WITH observations AS (
    SELECT provider.code AS provider_code, provider.name AS provider,
           tariff.plan_key, tariff.plan_name, tariff.cpu_cores::float8 AS cpu_cores,
           tariff.ram_mib::float8 / 1024.0 AS ram_gib,
           tariff.disk_gib::float8 AS disk_gib, tariff.region,
           price.observed_at, price.retrieved_at, price.source_url,
           price.source_kind, price.price_minor, price.currency::text AS currency,
           price.vat_included,
           lag(price.price_minor) OVER history AS previous_price_minor,
           lag(price.currency::text) OVER history AS previous_currency,
           lag(price.vat_included) OVER history AS previous_vat_included
    FROM market_intelligence.tariff_prices price
    JOIN market_intelligence.provider_tariffs tariff ON tariff.id = price.provider_tariff_id
    JOIN market_intelligence.providers provider ON provider.id = tariff.provider_id
    WHERE price.parse_status = 'complete' AND price.billing_period = 'month'
      AND price.price_kind IN ('regular', 'listed_monthly', 'official_catalog')
      AND price.price_minor IS NOT NULL
    WINDOW history AS (PARTITION BY price.provider_tariff_id, price.price_kind ORDER BY price.observed_at, price.id)
)
SELECT provider_code, provider, plan_key, plan_name, cpu_cores, ram_gib, disk_gib, region,
       observed_at, retrieved_at, source_url, source_kind,
       previous_price_minor, price_minor AS current_price_minor, currency,
       (price_minor::float8 / previous_price_minor::float8 - 1.0) * 100.0 AS change_percent
FROM observations
WHERE observed_at >= now() - interval '90 days'
  AND previous_currency = currency AND previous_price_minor > 0
  AND previous_vat_included IS NOT DISTINCT FROM vat_included
  AND price_minor <> previous_price_minor
ORDER BY observed_at DESC, provider, plan_name
