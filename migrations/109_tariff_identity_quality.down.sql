BEGIN;
DROP VIEW IF EXISTS public.financial_ledger;
DROP VIEW IF EXISTS market_intelligence.tariff_data_quality;
CREATE OR REPLACE VIEW market_intelligence.tariff_prices_rub AS
SELECT p.*, t.provider_id, t.plan_name,
       CASE WHEN p.currency = 'RUB' THEN 1::numeric
            WHEN p.currency = 'EUR' THEN fx.rate ELSE NULL END AS rub_rate,
       CASE WHEN p.currency = 'RUB' THEN p.observed_at::date
            WHEN p.currency = 'EUR' THEN fx.rate_on ELSE NULL END AS rub_rate_on,
       CASE WHEN p.price_minor IS NULL THEN NULL
            WHEN p.currency = 'RUB' THEN p.price_minor
            WHEN p.currency = 'EUR' AND fx.rate IS NOT NULL
                 THEN round(p.price_minor * fx.rate)::bigint
            ELSE NULL END AS price_rub_minor
FROM market_intelligence.tariff_prices p
JOIN market_intelligence.provider_tariffs t ON t.id = p.provider_tariff_id
LEFT JOIN market_intelligence.exchange_rates fx
  ON fx.base = p.currency AND fx.quote = 'RUB'
 AND fx.requested_on = p.observed_at::date;
DROP TRIGGER IF EXISTS services_match_tariff_product ON public.services;
DROP FUNCTION IF EXISTS public.match_service_tariff_product();
ALTER TABLE public.services DROP CONSTRAINT IF EXISTS services_tariff_product_fk;
DROP INDEX IF EXISTS public.services_tariff_product_idx;
ALTER TABLE public.services DROP COLUMN IF EXISTS tariff_product_id;
ALTER TABLE market_intelligence.provider_tariffs DROP CONSTRAINT IF EXISTS provider_tariffs_product_fk;
DROP INDEX IF EXISTS market_intelligence.provider_tariffs_product_idx;
ALTER TABLE market_intelligence.provider_tariffs DROP COLUMN IF EXISTS product_id;
DROP TABLE IF EXISTS market_intelligence.tariff_products;
COMMIT;
