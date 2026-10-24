-- Wayback timestamps are UTC. Match FX to the UTC capture date regardless of
-- the PostgreSQL session time zone (for example, Europe/Moscow).
CREATE OR REPLACE VIEW market_intelligence.tariff_prices_rub AS
SELECT p.*, t.provider_id, t.plan_name,
       CASE WHEN p.currency = 'RUB' THEN 1::numeric
            WHEN p.currency IN ('EUR', 'USD') THEN fx.rate ELSE NULL END AS rub_rate,
       CASE WHEN p.currency = 'RUB' THEN (p.observed_at AT TIME ZONE 'UTC')::date
            WHEN p.currency IN ('EUR', 'USD') THEN fx.rate_on ELSE NULL END AS rub_rate_on,
       CASE WHEN p.price_minor IS NULL THEN NULL
            WHEN p.currency = 'RUB' THEN p.price_minor
            WHEN p.currency IN ('EUR', 'USD') AND fx.rate IS NOT NULL
                 THEN round(p.price_minor * fx.rate)::bigint
            ELSE NULL END AS price_rub_minor
FROM market_intelligence.tariff_prices p
JOIN market_intelligence.provider_tariffs t ON t.id = p.provider_tariff_id
LEFT JOIN market_intelligence.exchange_rates fx
  ON fx.base = p.currency AND fx.quote = 'RUB'
 AND fx.requested_on = (p.observed_at AT TIME ZONE 'UTC')::date;
