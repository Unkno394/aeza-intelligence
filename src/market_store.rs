//! PostgreSQL persistence for competitor tariff observations.

use chrono::{DateTime, Utc};
use sqlx::{PgPool, Postgres, Transaction};

use crate::market::Offer;

#[derive(Debug, Clone)]
pub struct ObservationSource {
    pub kind: &'static str,
    pub observed_at: DateTime<Utc>,
    pub source_url: String,
    pub archive_url: Option<String>,
    pub cdx_digest: Option<String>,
    pub parser_version: &'static str,
}

pub async fn apply_schema(pool: &PgPool) -> Result<(), crate::Error> {
    crate::PgAdapter::from_pool(pool.clone()).migrate().await?;
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
        let normalized = normalize_offer(offer);
        save_offer(
            &mut transaction,
            provider_id,
            provider_code,
            source,
            &normalized,
        )
        .await?;
    }
    transaction.commit().await?;
    Ok(offers.len())
}

async fn save_offer(
    transaction: &mut Transaction<'_, Postgres>,
    provider_id: i64,
    provider_code: &str,
    source: &ObservationSource,
    offer: &Offer,
) -> Result<(), sqlx::Error> {
    let traffic_terms = matching_clause(
        &offer.raw_excerpt,
        &[
            "traffic",
            "bandwidth",
            "transfer",
            "трафик",
            "гбит",
            "mbit",
            "gbps",
        ],
    );
    let attributes = serde_json::json!({
        "parsed_page_details": offer.raw_excerpt,
        "price_kind": offer.price_kind,
        "vat_included": offer.vat_included,
        "normalized_details": {
            "cpu_cores": offer.cpu_cores,
            "ram_mib": offer.ram_mib,
            "disk_gib": offer.disk_gib,
            "disk_type": offer.disk_type.as_deref().map(normalize_disk_type),
            "region": offer.region,
            "traffic_terms": traffic_terms,
            "traffic_gib": traffic_terms.and_then(traffic_gib),
            "traffic_unlimited": traffic_terms.map(|value| {
                let value = value.to_lowercase();
                value.contains("unlimited") || value.contains("безлимит")
                    || value.contains("без ограничений")
            }),
            "payment_terms": offer.billing_period,
            "promotion_terms": matching_clause(&offer.raw_excerpt, &["discount", "promo", "sale", "скид", "акци"]),
            "restriction_terms": matching_clause(&offer.raw_excerpt, &["limit", "cap", "fair use", "огранич", "лимит"]),
        },
    })
    .to_string();
    let product_id: i64 = sqlx::query_scalar(
        "INSERT INTO market_intelligence.tariff_products (provider_id, plan_key, display_name) \
         VALUES ($1, $2, $3) ON CONFLICT (provider_id, plan_key) \
         DO UPDATE SET display_name = EXCLUDED.display_name RETURNING id",
    )
    .bind(provider_id)
    .bind(&offer.plan_key)
    .bind(&offer.plan_name)
    .fetch_one(&mut **transaction)
    .await?;
    if provider_code == "aeza" {
        sqlx::query(
            "UPDATE public.services SET tariff_product_id = $1 \
             WHERE tariff_product_id IS NULL AND lower(name) = lower($2)",
        )
        .bind(product_id)
        .bind(&offer.plan_key)
        .execute(&mut **transaction)
        .await?;
    }
    let tariff_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO market_intelligence.provider_tariffs
            (provider_id, plan_key, configuration_fingerprint, plan_name,
             cpu_cores, ram_mib, disk_gib, disk_type, region, attributes, product_id,
             first_observed_at)
           VALUES ($1, $2, $3, $4, $5::int4::numeric, $6,
                   $7::int4::numeric, $8, $9, $10::jsonb, $11, $12)
           ON CONFLICT (provider_id, plan_key, configuration_fingerprint)
           DO UPDATE SET first_observed_at = LEAST(
               provider_tariffs.first_observed_at,
               EXCLUDED.first_observed_at),
             attributes = provider_tariffs.attributes || EXCLUDED.attributes,
             product_id = EXCLUDED.product_id
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
    .bind(product_id)
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

fn normalize_disk_type(value: &str) -> String {
    let value = value.to_ascii_lowercase();
    if value.contains("nvme") {
        "NVMe".to_owned()
    } else if value.contains("ssd") {
        "SSD".to_owned()
    } else if value.contains("hdd") {
        "HDD".to_owned()
    } else {
        value.to_uppercase()
    }
}

fn normalize_offer(offer: &Offer) -> Offer {
    let mut normalized = offer.clone();
    normalized.disk_type = offer.disk_type.as_deref().map(normalize_disk_type);
    normalized.region = offer.region.as_deref().map(normalize_region);
    normalized
}

fn normalize_region(value: &str) -> String {
    match value.trim().to_lowercase().as_str() {
        "moscow" | "москва" | "ru-mow" => "RU-MOW".to_owned(),
        "saint petersburg" | "st. petersburg" | "санкт-петербург" | "ru-spe" => {
            "RU-SPE".to_owned()
        }
        "frankfurt" | "франкфурт" | "de-fra" => "DE-FRA".to_owned(),
        "helsinki" | "хельсинки" | "fi-hel" => "FI-HEL".to_owned(),
        "amsterdam" | "амстердам" | "nl-ams" => "NL-AMS".to_owned(),
        "london" | "лондон" | "gb-lon" => "GB-LON".to_owned(),
        "vienna" | "вена" | "at-vie" => "AT-VIE".to_owned(),
        "warsaw" | "варшава" | "pl-waw" => "PL-WAW".to_owned(),
        "paris" | "париж" | "fr-par" => "FR-PAR".to_owned(),
        "stockholm" | "стокгольм" | "se-sto" => "SE-STO".to_owned(),
        "charlotte" | "us-clt" => "US-CLT".to_owned(),
        _ => value.trim().to_owned(),
    }
}

fn traffic_gib(value: &str) -> Option<i32> {
    let lowercase = value.to_lowercase();
    if lowercase.contains("unlimited")
        || lowercase.contains("безлимит")
        || lowercase.contains("без ограничений")
    {
        return None;
    }
    let words: Vec<&str> = lowercase.split_whitespace().collect();
    for pair in words.windows(2) {
        let amount = pair[0]
            .trim_matches(|c: char| !c.is_ascii_digit())
            .parse::<i32>()
            .ok();
        let Some(amount) = amount else { continue };
        let unit = pair[1].trim_matches(|c: char| !c.is_alphanumeric());
        if matches!(unit, "tb" | "тб") {
            return amount.checked_mul(1024);
        }
        if matches!(unit, "gb" | "гб") {
            return Some(amount);
        }
    }
    None
}

fn matching_clause<'a>(text: &'a str, needles: &[&str]) -> Option<&'a str> {
    text.split(';').map(str::trim).find(|clause| {
        let lowercase = clause.to_lowercase();
        needles.iter().any(|needle| lowercase.contains(needle))
    })
}

#[cfg(test)]
mod tests {
    use super::{
        matching_clause, normalize_disk_type, normalize_offer, normalize_region, traffic_gib,
    };
    use crate::market::Offer;

    fn offer(raw_excerpt: &str) -> Offer {
        Offer {
            plan_key: "plan".into(),
            plan_name: "Plan".into(),
            cpu_cores: Some(4),
            ram_mib: Some(8192),
            disk_gib: Some(100),
            disk_type: Some("nvme ssd".into()),
            region: Some("Frankfurt".into()),
            price_minor: Some(1234),
            currency: Some("EUR".into()),
            billing_period: Some("month".into()),
            price_kind: "regular".into(),
            vat_included: Some(true),
            raw_excerpt: raw_excerpt.into(),
        }
    }

    #[test]
    fn normalized_details_preserve_source_terms_without_inventing_unknown_values() {
        let normalized = normalize_offer(&offer(
            "4 vCPU; 8 GB RAM; 100 GB NVMe SSD; Traffic: 5 TB; 20% discount; fair use limit applies",
        ));
        assert_eq!(normalized.disk_type.as_deref(), Some("NVMe"));
        assert_eq!(normalized.region.as_deref(), Some("DE-FRA"));
        let traffic = matching_clause(&normalized.raw_excerpt, &["traffic"]).unwrap();
        assert_eq!(traffic_gib(traffic), Some(5120));
        assert_eq!(
            matching_clause(&normalized.raw_excerpt, &["discount", "promo", "sale"]),
            Some("20% discount")
        );
        assert_eq!(
            matching_clause(&normalized.raw_excerpt, &["limit", "fair use"]),
            Some("fair use limit applies")
        );

        let unknown = Offer {
            disk_type: None,
            region: None,
            cpu_cores: None,
            ram_mib: None,
            disk_gib: None,
            price_minor: None,
            currency: None,
            billing_period: None,
            vat_included: None,
            ..offer("Specifications not published")
        };
        let normalized = normalize_offer(&unknown);
        assert!(normalized.cpu_cores.is_none());
        assert!(normalized.ram_mib.is_none());
        assert!(normalized.disk_gib.is_none());
        assert!(normalized.price_minor.is_none());
        assert!(normalized.region.is_none());
        assert!(normalized.disk_type.is_none());
    }

    #[test]
    fn traffic_and_location_normalization_handles_bounds_and_unlimited_terms() {
        assert_eq!(traffic_gib("Traffic: 25 GB"), Some(25));
        assert_eq!(traffic_gib("Traffic: 2 TB"), Some(2048));
        assert_eq!(traffic_gib("Unlimited traffic"), None);
        assert_eq!(traffic_gib("Traffic: 999999999999 TB"), None);
        assert_eq!(normalize_disk_type("enterprise NVMe SSD"), "NVMe");
        assert_eq!(normalize_disk_type("SATA SSD"), "SSD");
        assert_eq!(normalize_disk_type("HDD"), "HDD");
        assert_eq!(normalize_region("Москва"), "RU-MOW");
        assert_eq!(normalize_region("an unlisted region"), "an unlisted region");
    }
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
