//! PostgreSQL adapter: runs migrations and bulk-loads a [`Dataset`].

use std::time::Instant;

use chrono::{DateTime, Utc};

use serde_json::json;
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgPool, PgPoolOptions};
use sqlx::{Postgres, Transaction as PgTx};

use crate::Error;
use crate::model::{Account, ClientFeatures, Dataset, Service, Transaction};

/// `migrations/`, embedded at compile time.
pub static MIGRATOR: Migrator = sqlx::migrate!();

/// Rows per `INSERT ... SELECT FROM UNNEST(...)` statement.
const CHUNK_SIZE: usize = 10_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LoadReport {
    pub accounts: u64,
    pub services: u64,
    pub transactions: u64,
}

#[derive(Debug, Clone)]
pub struct PgAdapter {
    pool: PgPool,
}

impl PgAdapter {
    pub async fn connect(database_url: &str) -> Result<Self, Error> {
        let pool = PgPoolOptions::new()
            .max_connections(4)
            .connect(database_url)
            .await?;
        Ok(Self { pool })
    }

    pub fn from_pool(pool: PgPool) -> Self {
        Self { pool }
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    /// Applies pending migrations; already applied ones are skipped.
    pub async fn migrate(&self) -> Result<(), Error> {
        self.baseline_existing_client_tables().await?;
        MIGRATOR.run(&self.pool).await?;
        Ok(())
    }

    /// An older release created these three tables with plain SQL, without
    /// SQLx migration records. Validate and adapt that schema, then record only
    /// the corresponding creation migrations as already satisfied. Their SQL
    /// files and checksums remain unchanged for databases that used SQLx.
    async fn baseline_existing_client_tables(&self) -> Result<(), Error> {
        let existing: i64 = sqlx::query_scalar(
            "SELECT (to_regclass('public.accounts') IS NOT NULL)::int \
                  + (to_regclass('public.services') IS NOT NULL)::int \
                  + (to_regclass('public.transactions') IS NOT NULL)::int",
        )
        .fetch_one(&self.pool)
        .await?;
        if existing == 0 {
            return Ok(());
        }
        sqlx::raw_sql(include_str!("../migrations/100_legacy_client_schema.sql"))
            .execute(&self.pool)
            .await?;
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS _sqlx_migrations (\
                version bigint PRIMARY KEY, description text NOT NULL,\
                installed_on timestamptz NOT NULL DEFAULT now(), success boolean NOT NULL,\
                checksum bytea NOT NULL, execution_time bigint NOT NULL)",
        )
        .execute(&self.pool)
        .await?;
        for migration in MIGRATOR
            .iter()
            .filter(|m| (101..=103).contains(&m.version) && m.migration_type.is_up_migration())
        {
            sqlx::query(
                "INSERT INTO _sqlx_migrations (version, description, success, checksum, execution_time) \
                 VALUES ($1, $2, true, $3, 0) ON CONFLICT (version) DO NOTHING",
            )
            .bind(migration.version)
            .bind(&*migration.description)
            .bind(&*migration.checksum)
            .execute(&self.pool)
            .await?;
        }
        Ok(())
    }

    /// Reverts migrations newer than `target`, preserving the existing client
    /// and market tables (targets below 104 are refused).
    pub async fn rollback(&self, target: i64) -> Result<(), Error> {
        if target < 104 {
            return Err(Error::UnsafeRollback);
        }
        MIGRATOR.undo(&self.pool, target).await?;
        Ok(())
    }

    /// Risk feature vector of every account as of `as_of`, looking `horizon` ahead
    /// (a PostgreSQL interval such as `7 days`).
    pub async fn client_features(
        &self,
        as_of: DateTime<Utc>,
        horizon: &str,
    ) -> Result<Vec<ClientFeatures>, Error> {
        Ok(
            sqlx::query_as("SELECT * FROM public.client_features($1, $2::interval)")
                .bind(as_of)
                .bind(horizon)
                .fetch_all(&self.pool)
                .await?,
        )
    }

    /// Loads the dataset in a single transaction, with the same guarantees as `002_seed.sql`:
    /// the tables are locked, must be empty, and identity sequences are advanced past the
    /// loaded ids. On any error no data is written.
    ///
    /// Every inserted row is logged by the database triggers under one `requestId`, together
    /// with a `bulk_load` summary entry; a failed load leaves only a `failure` entry.
    pub async fn load(&self, data: &Dataset) -> Result<LoadReport, Error> {
        let request_id: String = sqlx::query_scalar("SELECT gen_random_uuid()::text")
            .fetch_one(&self.pool)
            .await?;
        let started = Instant::now();

        let result = self.load_in_tx(data, &request_id, started).await;
        if let Err(err) = &result {
            let metadata = json!({
                "accounts": data.accounts.len(),
                "services": data.services.len(),
                "transactions": data.transactions.len(),
            });
            // The load error is what the caller needs; failing to log it must not replace it.
            let _ = log_bulk_load(
                &self.pool,
                &request_id,
                started,
                Some(&err.to_string()),
                &metadata,
            )
            .await;
        }
        result
    }

    async fn load_in_tx(
        &self,
        data: &Dataset,
        request_id: &str,
        started: Instant,
    ) -> Result<LoadReport, Error> {
        let mut tx = self.pool.begin().await?;

        sqlx::query(
            "SELECT set_config('app.actor_type', 'system', true),
                    set_config('app.request_id', $1, true)",
        )
        .bind(request_id)
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            "LOCK TABLE public.accounts, public.services, public.transactions IN EXCLUSIVE MODE",
        )
        .execute(&mut *tx)
        .await?;

        let non_empty: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM public.accounts)
                 OR EXISTS (SELECT 1 FROM public.services)
                 OR EXISTS (SELECT 1 FROM public.transactions)",
        )
        .fetch_one(&mut *tx)
        .await?;
        if non_empty {
            return Err(Error::TablesNotEmpty);
        }

        let mut report = LoadReport::default();
        for chunk in data.accounts.chunks(CHUNK_SIZE) {
            report.accounts += insert_accounts(&mut tx, chunk).await?;
        }
        for chunk in data.services.chunks(CHUNK_SIZE) {
            report.services += insert_services(&mut tx, chunk).await?;
        }
        for chunk in data.transactions.chunks(CHUNK_SIZE) {
            report.transactions += insert_transactions(&mut tx, chunk).await?;
        }

        for table in ["accounts", "services", "transactions"] {
            sync_id_sequence(&mut tx, table).await?;
        }

        let metadata = json!({
            "accounts": report.accounts,
            "services": report.services,
            "transactions": report.transactions,
        });
        log_bulk_load(&mut *tx, request_id, started, None, &metadata).await?;

        tx.commit().await?;
        Ok(report)
    }
}

async fn log_bulk_load<'e, E>(
    executor: E,
    request_id: &str,
    started: Instant,
    error: Option<&str>,
    metadata: &serde_json::Value,
) -> Result<(), Error>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let duration_ms = i32::try_from(started.elapsed().as_millis()).unwrap_or(i32::MAX);
    sqlx::query(
        r#"INSERT INTO public.operation_logs ("actorType", action, outcome, error, "durationMs", "requestId", metadata)
           VALUES ('system', 'bulk_load', $1, $2, $3, $4::uuid, $5::jsonb)"#,
    )
    .bind(if error.is_some() { "failure" } else { "success" })
    .bind(error)
    .bind(duration_ms)
    .bind(request_id)
    .bind(metadata.to_string())
    .execute(executor)
    .await?;
    Ok(())
}

async fn insert_accounts(tx: &mut PgTx<'_, Postgres>, rows: &[Account]) -> Result<u64, Error> {
    let result = sqlx::query(
        r#"INSERT INTO public.accounts ("id", "email", "balance", "withdrawBalance", "totalReplenished", "bonusBalance", "bonusState")
           SELECT * FROM UNNEST($1::int8[], $2::text[], $3::int8[], $4::int8[], $5::int8[], $6::int8[], $7::text[])"#,
    )
    .bind(rows.iter().map(|r| r.id).collect::<Vec<_>>())
    .bind(rows.iter().map(|r| r.email.as_str()).collect::<Vec<_>>())
    .bind(rows.iter().map(|r| r.balance).collect::<Vec<_>>())
    .bind(rows.iter().map(|r| r.withdraw_balance).collect::<Vec<_>>())
    .bind(rows.iter().map(|r| r.total_replenished).collect::<Vec<_>>())
    .bind(rows.iter().map(|r| r.bonus_balance).collect::<Vec<_>>())
    .bind(rows.iter().map(|r| r.bonus_state).collect::<Vec<_>>())
    .execute(&mut **tx)
    .await?;
    Ok(result.rows_affected())
}

async fn insert_services(tx: &mut PgTx<'_, Postgres>, rows: &[Service]) -> Result<u64, Error> {
    let result = sqlx::query(
        r#"INSERT INTO public.services ("id", "name", "ownerId", "paymentTerm", "autoProlong", "createdAt", "expiresAt", "status", "price")
           SELECT * FROM UNNEST($1::int8[], $2::text[], $3::int8[], $4::text[], $5::bool[], $6::timestamptz[], $7::timestamptz[], $8::text[], $9::int8[])"#,
    )
    .bind(rows.iter().map(|r| r.id).collect::<Vec<_>>())
    .bind(rows.iter().map(|r| r.name.as_str()).collect::<Vec<_>>())
    .bind(rows.iter().map(|r| r.owner_id).collect::<Vec<_>>())
    .bind(rows.iter().map(|r| r.payment_term).collect::<Vec<_>>())
    .bind(rows.iter().map(|r| r.auto_prolong).collect::<Vec<_>>())
    .bind(rows.iter().map(|r| r.created_at).collect::<Vec<_>>())
    .bind(rows.iter().map(|r| r.expires_at).collect::<Vec<_>>())
    .bind(rows.iter().map(|r| r.status).collect::<Vec<_>>())
    .bind(rows.iter().map(|r| r.price).collect::<Vec<_>>())
    .execute(&mut **tx)
    .await?;
    Ok(result.rows_affected())
}

async fn insert_transactions(
    tx: &mut PgTx<'_, Postgres>,
    rows: &[Transaction],
) -> Result<u64, Error> {
    let result = sqlx::query(
        r#"INSERT INTO public.transactions ("id", "amount", "status", "performedAt", "createdAt", "type", "serviceId")
           SELECT * FROM UNNEST($1::int8[], $2::int4[], $3::text[], $4::timestamptz[], $5::timestamptz[], $6::text[], $7::int8[])"#,
    )
    .bind(rows.iter().map(|r| r.id).collect::<Vec<_>>())
    .bind(rows.iter().map(|r| r.amount).collect::<Vec<_>>())
    .bind(rows.iter().map(|r| r.status).collect::<Vec<_>>())
    .bind(rows.iter().map(|r| r.performed_at).collect::<Vec<_>>())
    .bind(rows.iter().map(|r| r.created_at).collect::<Vec<_>>())
    .bind(rows.iter().map(|r| r.kind).collect::<Vec<_>>())
    .bind(rows.iter().map(|r| r.service_id).collect::<Vec<_>>())
    .execute(&mut **tx)
    .await?;
    Ok(result.rows_affected())
}

/// Moves the identity sequence past explicitly inserted ids, so later inserts without an id
/// don't collide. Leaves the sequence untouched when the table is empty.
async fn sync_id_sequence(tx: &mut PgTx<'_, Postgres>, table: &str) -> Result<(), Error> {
    let qualified = format!("public.{table}");
    sqlx::query(&format!(
        "SELECT setval(pg_get_serial_sequence($1, 'id'), max_id, true)
         FROM (SELECT max(id) AS max_id FROM {qualified}) m
         WHERE max_id IS NOT NULL"
    ))
    .bind(&qualified)
    .execute(&mut **tx)
    .await?;
    Ok(())
}
