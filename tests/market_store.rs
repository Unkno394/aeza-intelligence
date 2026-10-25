//! Persistence checks for market observation identity and repeatable imports.
//!
//! Use only with a dedicated, empty disposable database through
//! `MARKET_TEST_DATABASE_URL`.

use aeza_intelligence::{
    PgAdapter,
    market::Offer,
    market_store::{ObservationSource, save_offers},
};
use chrono::{TimeZone, Utc};
use sqlx::{Row, postgres::PgPoolOptions};

#[tokio::test]
async fn repeated_observations_are_idempotent_and_price_changes_keep_history() {
    let Some(database_url) = std::env::var_os("MARKET_TEST_DATABASE_URL") else {
        eprintln!("market persistence test skipped: MARKET_TEST_DATABASE_URL is unset");
        return;
    };
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect(database_url.to_str().expect("database URL is UTF-8"))
        .await
        .expect("connect to disposable market database");
    PgAdapter::from_pool(pool.clone()).migrate().await.unwrap();
    let existing: i64 =
        sqlx::query_scalar("SELECT count(*) FROM market_intelligence.tariff_prices")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        existing, 0,
        "test requires a dedicated empty market database"
    );

    let observed_at = Utc.with_ymd_and_hms(2026, 10, 20, 12, 0, 0).unwrap();
    let source = ObservationSource {
        kind: "live",
        observed_at,
        source_url: "https://market-fixture.invalid/vps".into(),
        archive_url: None,
        cdx_digest: None,
        parser_version: "test-parser-v1",
    };
    let offer = |price_minor| Offer {
        plan_key: "test-vps".into(),
        plan_name: "Test VPS".into(),
        cpu_cores: Some(4),
        ram_mib: Some(8192),
        disk_gib: Some(100),
        disk_type: Some("nvme".into()),
        region: Some("Frankfurt".into()),
        price_minor,
        currency: Some("EUR".into()),
        billing_period: Some("month".into()),
        price_kind: "regular".into(),
        vat_included: Some(true),
        raw_excerpt: "4 vCPU; 8 GB RAM; 100 GB NVMe; Traffic: 5 TB; 10% discount".into(),
    };

    assert_eq!(
        save_offers(&pool, "contabo", &source, &[offer(Some(1000))])
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        save_offers(&pool, "contabo", &source, &[offer(Some(1000))])
            .await
            .unwrap(),
        1
    );
    let observations: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM market_intelligence.tariff_prices prices \
         JOIN market_intelligence.provider_tariffs tariffs ON tariffs.id = prices.provider_tariff_id \
         JOIN market_intelligence.providers providers ON providers.id = tariffs.provider_id \
         WHERE providers.code = 'contabo'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        observations, 1,
        "same source, date, plan and price are idempotent"
    );

    let changed_source = ObservationSource {
        observed_at: observed_at + chrono::Duration::days(1),
        ..source.clone()
    };
    save_offers(&pool, "contabo", &changed_source, &[offer(Some(1250))])
        .await
        .unwrap();
    let prices: Vec<i64> = sqlx::query_scalar(
        "SELECT prices.price_minor FROM market_intelligence.tariff_prices prices \
         JOIN market_intelligence.provider_tariffs tariffs ON tariffs.id = prices.provider_tariff_id \
         JOIN market_intelligence.providers providers ON providers.id = tariffs.provider_id \
         WHERE providers.code = 'contabo' ORDER BY prices.observed_at",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        prices,
        [1000, 1250],
        "a changed price adds a dated observation"
    );

    let incomplete_source = ObservationSource {
        observed_at: observed_at + chrono::Duration::days(2),
        ..source.clone()
    };
    save_offers(&pool, "contabo", &incomplete_source, &[offer(None)])
        .await
        .unwrap();

    let incomplete: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM market_intelligence.tariff_prices prices \
         JOIN market_intelligence.provider_tariffs tariffs ON tariffs.id = prices.provider_tariff_id \
         JOIN market_intelligence.providers providers ON providers.id = tariffs.provider_id \
         WHERE providers.code = 'contabo' AND prices.price_minor IS NULL \
           AND prices.parse_status = 'incomplete'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        incomplete, 1,
        "failed price parsing is recorded as incomplete with a null amount"
    );
    let fabricated_prices: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM market_intelligence.tariff_prices prices \
         JOIN market_intelligence.provider_tariffs tariffs ON tariffs.id = prices.provider_tariff_id \
         JOIN market_intelligence.providers providers ON providers.id = tariffs.provider_id \
         WHERE providers.code = 'contabo' AND prices.observed_at = $1 AND prices.price_minor IS NOT NULL",
    )
    .bind(incomplete_source.observed_at)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        fabricated_prices, 0,
        "parse failure cannot become a made-up price"
    );

    let details: serde_json::Value = sqlx::query_scalar(
        "SELECT tariffs.attributes FROM market_intelligence.provider_tariffs tariffs \
         JOIN market_intelligence.providers providers ON providers.id = tariffs.provider_id \
         WHERE providers.code = 'contabo' AND tariffs.plan_key = 'test-vps'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(details["normalized_details"]["region"], "DE-FRA");
    assert_eq!(details["normalized_details"]["disk_type"], "NVMe");
    assert_eq!(details["normalized_details"]["traffic_gib"], 5120);
    assert_eq!(
        details["normalized_details"]["promotion_terms"],
        "10% discount"
    );

    // Catalog history keeps multiple increases, while the behavior study uses
    // the latest event and measures the exact 30-day windows around it.
    let account_id = 99_900_001_i64;
    let service_id = 99_910_001_i64;
    let changed_at = Utc::now() - chrono::Duration::days(65);
    sqlx::query(
        "INSERT INTO public.accounts (id, email, balance, \"totalReplenished\") \
         VALUES ($1, 'price-behavior@real.test', 10000, 10000)",
    )
    .bind(account_id)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO public.services \
         (id, name, \"ownerId\", \"paymentTerm\", \"autoProlong\", \"createdAt\", \"expiresAt\", status, price) \
         VALUES ($1, 'behavior-vps', $2, 'month', true, $3, $4, 'active', 1200)",
    )
    .bind(service_id)
    .bind(account_id)
    .bind(changed_at - chrono::Duration::days(100))
    .bind(changed_at + chrono::Duration::days(20))
    .execute(&pool)
    .await
    .unwrap();
    let behavior_offer = |price_minor| Offer {
        plan_key: "behavior-vps".into(),
        plan_name: "Behavior VPS".into(),
        cpu_cores: Some(2),
        ram_mib: Some(4096),
        disk_gib: Some(80),
        disk_type: Some("NVMe".into()),
        region: Some("Frankfurt".into()),
        price_minor: Some(price_minor),
        currency: Some("RUB".into()),
        billing_period: Some("month".into()),
        price_kind: "regular".into(),
        vat_included: Some(true),
        raw_excerpt: "2 vCPU; 4 GB RAM; 80 GB NVMe; Traffic: unlimited".into(),
    };
    for (days_before_change, price_minor) in [(10, 1000), (5, 1200), (0, 1500)] {
        let source = ObservationSource {
            kind: "live",
            observed_at: changed_at - chrono::Duration::days(days_before_change),
            source_url: "https://aeza.test/behavior-vps".into(),
            archive_url: None,
            cdx_digest: None,
            parser_version: "behavior-fixture-v1",
        };
        save_offers(&pool, "aeza", &source, &[behavior_offer(price_minor)])
            .await
            .unwrap();
    }
    sqlx::query(
        "INSERT INTO public.transactions \
         (id, amount, status, \"performedAt\", \"createdAt\", type, \"serviceId\") \
         VALUES ($1, 500, 'performed', $2, $2 - interval '1 minute', 'prolong', $3), \
                ($4, 500, 'performed', $5, $5 - interval '1 minute', 'prolong', $3)",
    )
    .bind(99_920_001_i64)
    .bind(changed_at - chrono::Duration::days(15))
    .bind(service_id)
    .bind(99_920_002_i64)
    .bind(changed_at + chrono::Duration::days(15))
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO public.operation_logs \
         (\"occurredAt\", \"actorType\", action, entity, \"entityId\", before, after, outcome, metadata) \
         VALUES ($1, 'system', 'update', 'service', $2, '{}', '{}', 'success', '{}'), \
                ($3, 'system', 'update', 'service', $2, \
                 '{\"status\":\"active\",\"autoProlong\":true}', \
                 '{\"status\":\"prolong_wait\",\"autoProlong\":false}', \
                 'success', '{}')",
    )
    .bind(changed_at - chrono::Duration::days(31))
    .bind(service_id)
    .bind(changed_at + chrono::Duration::days(20))
    .execute(&pool)
    .await
    .unwrap();

    let behavior_rows = sqlx::query(include_str!("../src/sql/price_behavior.sql"))
        .fetch_all(&pool)
        .await
        .unwrap();
    assert_eq!(
        behavior_rows.len(),
        2,
        "latest price change has before/after windows"
    );
    for row in &behavior_rows {
        assert_eq!(row.get::<String, _>("plan_key"), "behavior-vps");
        assert_eq!(row.get::<i64, _>("previous_minor"), 1200);
        assert_eq!(row.get::<i64, _>("current_minor"), 1500);
        assert_eq!(row.get::<i64, _>("services"), 1);
        assert_eq!(row.get::<i64, _>("accounts"), 1);
        assert_eq!(row.get::<bool, _>("complete_followup"), true);
        assert_eq!(
            row.get::<Option<bool>, _>("complete_event_coverage"),
            Some(true)
        );
        let period = row.get::<String, _>("period");
        if period == "before" {
            assert_eq!(row.get::<i64, _>("performed_renewals"), 1);
            assert_eq!(row.get::<i64, _>("renewal_revenue_minor"), 500);
            assert_eq!(row.get::<i64, _>("prolong_wait_entries"), 0);
        } else {
            assert_eq!(period, "after");
            assert_eq!(row.get::<i64, _>("performed_renewals"), 1);
            assert_eq!(row.get::<i64, _>("renewal_revenue_minor"), 500);
            assert_eq!(row.get::<i64, _>("prolong_wait_entries"), 1);
            assert_eq!(row.get::<i64, _>("prolong_wait_services"), 1);
        }
    }

    pool.close().await;
}
