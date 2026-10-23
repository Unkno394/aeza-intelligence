//! PostgreSQL persistence for competitor tariff observations.

use chrono::{DateTime, Utc};
use sqlx::{PgPool, Postgres, Transaction};

use crate::market::Offer;

pub const SCHEMA_SQL: &str = include_str!("../migrations/001_market_intelligence.sql");

#[derive(Debug, Clone)]
pub struct ObservationSource {
    pub kind: &'static str,
    pub observed_at: DateTime<Utc>,
    pub source_url: String,
    pub archive_url: Option<String>,
    pub cdx_digest: Option<String>,
    pub parser_version: &'static str,
}

pub async fn apply_schema(pool: &PgPool) -> Result<(), sqlx::Error> {
    sqlx::raw_sql(SCHEMA_SQL).execute(pool).await?;
    Ok(())
}

pub async fn save_offers(
    pool: &PgPool,
    provider_code: &str,
    source: &ObservationSource,
    offers: &[Offer],
) -> Result<usize, sqlx::Error> {
    let provider_id: i64 =
        sqlx::query_scalar("SELECT id FROM market_intelligence.providers WHERE code = $1")
            .bind(provider_code)
            .fetch_one(pool)
            .await?;
    let mut transaction = pool.begin().await?;
    for offer in offers {
        save_offer(&mut transaction, provider_id, source, offer).await?;
    }
    transaction.commit().await?;
    Ok(offers.len())
}

async fn save_offer(
    transaction: &mut Transaction<'_, Postgres>,
    provider_id: i64,
    source: &ObservationSource,
    offer: &Offer,
) -> Result<(), sqlx::Error> {
    let attributes = "{}";
    let tariff_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO market_intelligence.provider_tariffs
            (provider_id, plan_key, configuration_fingerprint, plan_name,
             cpu_cores, ram_mib, disk_gib, disk_type, region, attributes,
             first_observed_at)
           VALUES ($1, $2, $3, $4, $5::int4::numeric, $6,
                   $7::int4::numeric, $8, $9, $10::jsonb, $11)
           ON CONFLICT (provider_id, plan_key, configuration_fingerprint)
           DO UPDATE SET first_observed_at = LEAST(
               provider_tariffs.first_observed_at,
               EXCLUDED.first_observed_at)
           RETURNING id"#,
    )
    .bind(provider_id)
    .bind(&offer.plan_key)
    .bind(offer.configuration_fingerprint())
    .bind(&offer.plan_name)
    .bind(offer.cpu_cores)
    .bind(offer.ram_mib)
    .bind(offer.disk_gib)
    .bind(&offer.disk_type)
    .bind(&offer.region)
    .bind(attributes)
    .bind(source.observed_at)
    .fetch_one(&mut **transaction)
    .await?;

    let observation_key = format!(
        "{}|{}|{}|{}|{}|{}|{}",
        source.kind,
        source.source_url,
        source.observed_at.timestamp_millis(),
        offer.plan_key,
        offer.billing_period.as_deref().unwrap_or("?"),
        offer.currency.as_deref().unwrap_or("?"),
        offer.price_kind
    );
    sqlx::query(
        r#"INSERT INTO market_intelligence.tariff_prices
            (provider_tariff_id, observation_key, source_kind, observed_at,
             price_minor, currency, billing_period, price_kind, vat_included,
             source_url, archive_url, cdx_digest, parser_version, parse_status,
             raw_excerpt)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12,
                   $13, $14, $15)
           ON CONFLICT (provider_tariff_id, observation_key)
           DO UPDATE SET price_minor = EXCLUDED.price_minor,
                         currency = EXCLUDED.currency,
                         billing_period = EXCLUDED.billing_period,
                         vat_included = EXCLUDED.vat_included,
                         parser_version = EXCLUDED.parser_version,
                         parse_status = EXCLUDED.parse_status,
                         raw_excerpt = EXCLUDED.raw_excerpt"#,
    )
    .bind(tariff_id)
    .bind(observation_key)
    .bind(source.kind)
    .bind(source.observed_at)
    .bind(offer.price_minor)
    .bind(&offer.currency)
    .bind(&offer.billing_period)
    .bind(&offer.price_kind)
    .bind(offer.vat_included)
    .bind(&source.source_url)
    .bind(&source.archive_url)
    .bind(&source.cdx_digest)
    .bind(source.parser_version)
    .bind(if offer.is_complete() {
        "complete"
    } else {
        "incomplete"
    })
    .bind(&offer.raw_excerpt)
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

pub async fn count_observations(pool: &PgPool, provider_code: &str) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar(
        r#"SELECT count(*)
           FROM market_intelligence.tariff_prices AS prices
           JOIN market_intelligence.provider_tariffs AS tariffs
             ON tariffs.id = prices.provider_tariff_id
           JOIN market_intelligence.providers AS providers
             ON providers.id = tariffs.provider_id
           WHERE providers.code = $1"#,
    )
    .bind(provider_code)
    .fetch_one(pool)
    .await
}
