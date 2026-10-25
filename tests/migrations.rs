//! Migrations and the client data loader on a real PostgreSQL. Each test gets
//! a fresh database from `#[sqlx::test]`; DATABASE_URL must point to a server
//! where the user may create databases.

use aeza_intelligence::{Dataset, Error, PgAdapter, adapter::MIGRATOR};
use sqlx::PgPool;

fn fixture() -> Dataset {
    Dataset::from_path(concat!(env!("CARGO_MANIFEST_DIR"), "/scripts/fixtures/data.json")).unwrap()
}

#[test]
fn versions_are_unique_and_without_gaps() {
    let mut versions: Vec<i64> = MIGRATOR
        .iter()
        .filter(|migration| migration.migration_type.is_up_migration())
        .map(|migration| migration.version)
        .collect();
    let count = versions.len();
    versions.dedup();
    assert_eq!(versions.len(), count, "two migrations share a version");
    // Market migrations are 001..; client migrations continue from 100.
    for series in [1..100, 100..1000] {
        let numbers: Vec<i64> = versions.iter().copied().filter(|v| series.contains(v)).collect();
        let expected: Vec<i64> = (series.start..).take(numbers.len()).collect();
        assert_eq!(numbers, expected, "the {}.. series has a gap", series.start);
    }
}

#[test]
fn every_reversible_migration_has_a_down_script() {
    let ups = MIGRATOR.iter().filter(|m| m.migration_type.is_up_migration() && m.migration_type.is_reversible());
    for up in ups {
        let has_down = MIGRATOR
            .iter()
            .any(|m| m.version == up.version && m.migration_type.is_down_migration());
        assert!(has_down, "migration {} has no .down.sql", up.version);
    }
}

#[sqlx::test(migrations = false)]
async fn migrate_is_idempotent_on_an_empty_database(pool: PgPool) {
    let adapter = PgAdapter::from_pool(pool);
    adapter.migrate().await.unwrap();
    adapter.migrate().await.unwrap();
    let applied: i64 = sqlx::query_scalar("SELECT count(*) FROM _sqlx_migrations WHERE success")
        .fetch_one(adapter.pool())
        .await
        .unwrap();
    let expected = MIGRATOR.iter().filter(|m| m.migration_type.is_up_migration()).count();
    assert_eq!(applied as usize, expected);

    let required = [
        "public.accounts",
        "public.services",
        "public.transactions",
        "public.admins",
        "public.admin_sessions",
        "public.operation_logs",
        "public.client_snapshots",
        "public.client_settings",
        "public.client_settings_history",
        "public.financial_ledger",
        "market_intelligence.providers",
        "market_intelligence.provider_tariffs",
        "market_intelligence.tariff_products",
        "market_intelligence.tariff_prices",
        "market_intelligence.tariff_data_quality",
        "market_intelligence.search_brands",
        "market_intelligence.search_interest",
        "market_intelligence.search_interest_geo",
        "market_intelligence.exchange_rates",
    ];
    let missing: Vec<String> = sqlx::query_scalar(
        "SELECT name FROM unnest($1::text[]) AS required(name) WHERE to_regclass(name) IS NULL",
    )
    .bind(&required[..])
    .fetch_all(adapter.pool())
    .await
    .unwrap();
    assert!(missing.is_empty(), "missing after migrate: {missing:?}");
}

#[sqlx::test(migrations = false)]
async fn every_migration_after_104_rolls_back_and_reapplies(pool: PgPool) {
    let adapter = PgAdapter::from_pool(pool);
    adapter.migrate().await.unwrap();
    adapter.rollback(104).await.unwrap();
    let latest: i64 = sqlx::query_scalar("SELECT max(version) FROM _sqlx_migrations")
        .fetch_one(adapter.pool())
        .await
        .unwrap();
    assert_eq!(latest, 104);
    adapter.migrate().await.unwrap();
}

#[sqlx::test(migrations = false)]
async fn rollback_below_104_is_refused(pool: PgPool) {
    let adapter = PgAdapter::from_pool(pool);
    adapter.migrate().await.unwrap();
    assert!(matches!(adapter.rollback(103).await, Err(Error::UnsafeRollback)));
}

#[sqlx::test(migrations = false)]
async fn a_partial_legacy_client_schema_is_not_migrated(pool: PgPool) {
    sqlx::query("CREATE TABLE public.accounts (id bigint PRIMARY KEY)")
        .execute(&pool)
        .await
        .unwrap();
    let error = PgAdapter::from_pool(pool).migrate().await.unwrap_err();
    assert!(error.to_string().contains("Partial legacy client schema"), "{error}");
}

#[sqlx::test(migrator = "aeza_intelligence::adapter::MIGRATOR")]
async fn the_fixture_loads_once_into_empty_tables(pool: PgPool) {
    let adapter = PgAdapter::from_pool(pool);
    assert!(!adapter.has_client_data().await.unwrap());
    let report = adapter.load(&fixture()).await.unwrap();
    assert_eq!((report.accounts, report.services, report.transactions), (10, 30, 100));
    assert!(adapter.has_client_data().await.unwrap());
    assert!(matches!(adapter.load(&fixture()).await, Err(Error::TablesNotEmpty)));

    // New rows continue after the loaded IDs instead of colliding with them.
    let id: i64 = sqlx::query_scalar(
        "INSERT INTO public.accounts (email) VALUES ('new@example.test') RETURNING id::bigint",
    )
    .fetch_one(adapter.pool())
    .await
    .unwrap();
    assert!(id > 10, "{id}");
}
