//! PostgreSQL adapter: applies the schema and bulk-loads a [`Dataset`].

use sqlx::postgres::{PgPool, PgPoolOptions};
use sqlx::{Postgres, Transaction as PgTx};

use crate::Error;
use crate::model::{Account, Dataset, Service, Transaction};

/// `scripts/001_schema.sql`, embedded so the binary doesn't depend on the working directory.
pub const SCHEMA_SQL: &str = include_str!("../scripts/001_schema.sql");

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

    /// Creates enum types, tables and indexes if they don't exist yet.
    pub async fn apply_schema(&self) -> Result<(), Error> {
        sqlx::raw_sql(SCHEMA_SQL).execute(&self.pool).await?;
        Ok(())
    }

    /// Loads the dataset in a single transaction, with the same guarantees as `002_seed.sql`:
    /// the tables are locked, must be empty, and identity sequences are advanced past the
    /// loaded ids. On any error nothing is written.
    pub async fn load(&self, data: &Dataset) -> Result<LoadReport, Error> {
        let mut tx = self.pool.begin().await?;

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

        tx.commit().await?;
        Ok(report)
    }
}

async fn insert_accounts(tx: &mut PgTx<'_, Postgres>, rows: &[Account]) -> Result<u64, Error> {
    let result = sqlx::query(
        r#"INSERT INTO public.accounts ("id", "email", "balance", "withdrawBalance", "totalReplenished", "bonusBalance", "bonusState")
           SELECT * FROM UNNEST($1::int4[], $2::text[], $3::int8[], $4::int8[], $5::int8[], $6::int8[], $7::public.bonus_state[])"#,
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
           SELECT * FROM UNNEST($1::int4[], $2::text[], $3::int4[], $4::public.payment_term[], $5::bool[], $6::timestamptz[], $7::timestamptz[], $8::public.service_status[], $9::int8[])"#,
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
           SELECT * FROM UNNEST($1::int4[], $2::int8[], $3::public.transaction_status[], $4::timestamptz[], $5::timestamptz[], $6::public.transaction_type[], $7::int4[])"#,
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
