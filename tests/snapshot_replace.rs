use aeza_intelligence::model::{
    Account, BonusState, PaymentTerm, Service, ServiceStatus, Transaction, TransactionStatus,
    TransactionType,
};
use aeza_intelligence::{Dataset, PgAdapter};
use chrono::{Duration, Utc};
use sqlx::{PgPool, Row, postgres::PgPoolOptions};

fn snapshot(id: i64, email: &str) -> Dataset {
    let created_at = Utc::now() - Duration::days(1);
    Dataset {
        accounts: vec![Account {
            id,
            email: email.into(),
            balance: 1_000,
            withdraw_balance: 200,
            total_replenished: 5_000,
            bonus_balance: 300,
            bonus_state: BonusState::Unlocked,
        }],
        services: vec![Service {
            id,
            name: format!("vps-{id}"),
            owner_id: id,
            payment_term: PaymentTerm::Month,
            auto_prolong: true,
            created_at,
            expires_at: created_at + Duration::days(30),
            status: ServiceStatus::Active,
            price: 500,
        }],
        transactions: vec![Transaction {
            id,
            amount: 500,
            status: TransactionStatus::Performed,
            performed_at: Some(created_at),
            created_at,
            kind: TransactionType::Order,
            service_id: id,
        }],
    }
}

async fn ids(pool: &PgPool) -> Vec<i64> {
    sqlx::query("SELECT id FROM public.accounts ORDER BY id")
        .fetch_all(pool)
        .await
        .unwrap()
        .into_iter()
        .map(|row| row.get("id"))
        .collect()
}

/// This is deliberately opt-in because it owns and clears all client rows in
/// the target database. Use a disposable database:
/// TEST_DATABASE_URL=postgres://... cargo test --test snapshot_replace
#[tokio::test]
async fn first_load_replacement_validation_and_database_failure_are_atomic() {
    let Some(database_url) = std::env::var_os("TEST_DATABASE_URL") else {
        eprintln!("snapshot replacement integration test skipped: TEST_DATABASE_URL is unset");
        return;
    };
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect(
            database_url
                .to_str()
                .expect("TEST_DATABASE_URL must be UTF-8"),
        )
        .await
        .unwrap();
    let adapter = PgAdapter::from_pool(pool.clone());
    adapter.migrate().await.unwrap();
    adapter
        .migrate()
        .await
        .expect("reapplying completed migrations is a supported no-op");
    sqlx::raw_sql(
        "DELETE FROM public.transactions; DELETE FROM public.services; DELETE FROM public.accounts",
    )
    .execute(&pool)
    .await
    .unwrap();

    // First upload into empty tables.
    let first = snapshot(101, "first@replace.test");
    assert_eq!(adapter.replace(&first).await.unwrap().accounts, 1);
    assert_eq!(ids(&pool).await, [101]);

    // Database constraints also protect the tables from writes that bypass
    // the application-level Dataset validator.
    assert!(
        sqlx::query("INSERT INTO public.accounts (id, email) VALUES (102, 'first@replace.test')",)
            .execute(&pool)
            .await
            .is_err()
    );
    assert!(sqlx::query(
        "INSERT INTO public.accounts (id, email, balance) VALUES (102, 'negative@replace.test', -1)",
    )
    .execute(&pool)
    .await
    .is_err());
    assert!(sqlx::query(
        "INSERT INTO public.services (id, name, \"ownerId\", \"paymentTerm\", status, \"expiresAt\", price) \
         VALUES (102, 'orphan-service', 999, 'month', 'active', now(), 100)",
    )
    .execute(&pool)
    .await
    .is_err());
    sqlx::query(
        "INSERT INTO public.services (id, name, \"ownerId\", \"paymentTerm\", status, \"expiresAt\", price) \
         VALUES (103, 'valid-service', 101, 'month', 'active', now(), 100)",
    )
    .execute(&pool)
    .await
    .unwrap();
    assert!(sqlx::query(
        "INSERT INTO public.transactions (id, amount, status, \"performedAt\", type, \"serviceId\") \
         VALUES (102, -1, 'not_performed', NULL, 'order', 103)",
    )
    .execute(&pool)
    .await
    .is_err());
    assert!(sqlx::query(
        "INSERT INTO public.transactions (id, amount, status, \"performedAt\", type, \"serviceId\") \
         VALUES (104, 100, 'not_performed', NULL, 'order', 999)",
    )
    .execute(&pool)
    .await
    .is_err());
    assert_eq!(
        ids(&pool).await,
        [101],
        "constraint failures leave rows unchanged"
    );

    // A second upload replaces, rather than appends to, the current snapshot.
    let second = snapshot(202, "second@replace.test");
    assert_eq!(adapter.replace(&second).await.unwrap().transactions, 1);
    assert_eq!(ids(&pool).await, [202]);
    let replay = adapter.replace(&second).await.unwrap();
    assert_eq!(replay.accounts, 1);
    assert_eq!(replay.services, 1);
    assert_eq!(replay.transactions, 1);
    assert_eq!(
        ids(&pool).await,
        [202],
        "reloading the same source is idempotent"
    );
    let row_counts: (i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM public.services), (SELECT count(*) FROM public.transactions)",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(row_counts, (1, 1));

    // Invalid foreign-key structure is rejected before old rows are deleted.
    let mut invalid = snapshot(303, "invalid@replace.test");
    invalid.services[0].owner_id = 999;
    assert!(adapter.replace(&invalid).await.is_err());
    assert_eq!(ids(&pool).await, [202]);

    // A database error after DELETE exercises transaction rollback itself.
    sqlx::raw_sql(
        "CREATE OR REPLACE FUNCTION public.aeza_test_reject_snapshot_account() RETURNS trigger \
         LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'forced replacement failure'; END $$; \
         CREATE TRIGGER reject_snapshot_account BEFORE INSERT ON public.accounts \
         FOR EACH ROW EXECUTE FUNCTION public.aeza_test_reject_snapshot_account()",
    )
    .execute(&pool)
    .await
    .unwrap();
    let failed = snapshot(404, "rollback@replace.test");
    assert!(adapter.replace(&failed).await.is_err());
    assert_eq!(ids(&pool).await, [202]);
    sqlx::raw_sql(
        "DROP TRIGGER reject_snapshot_account ON public.accounts; \
         DROP FUNCTION public.aeza_test_reject_snapshot_account()",
    )
    .execute(&pool)
    .await
    .unwrap();
}
