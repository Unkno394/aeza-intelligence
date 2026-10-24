-- Stable product identity across price/configuration observations and client renewals.
BEGIN;

INSERT INTO market_intelligence.providers (code, name, website_url, tariff_page_urls)
VALUES
    ('aeza', 'Aéza', 'https://aeza.net', ARRAY['https://aeza.net/ru/virtual-servers']),
    ('contabo', 'Contabo', 'https://contabo.com', ARRAY['https://contabo.com/en/pricing/'])
ON CONFLICT (code) DO NOTHING;

CREATE TABLE IF NOT EXISTS market_intelligence.tariff_products (
    id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    provider_id bigint NOT NULL REFERENCES market_intelligence.providers(id),
    plan_key text NOT NULL,
    display_name text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (provider_id, plan_key),
    CHECK (plan_key <> ''),
    CHECK (display_name <> '')
);

ALTER TABLE market_intelligence.provider_tariffs
    ADD COLUMN IF NOT EXISTS product_id bigint;

INSERT INTO market_intelligence.tariff_products (provider_id, plan_key, display_name)
SELECT provider_id, plan_key, min(plan_name)
FROM market_intelligence.provider_tariffs
GROUP BY provider_id, plan_key
ON CONFLICT (provider_id, plan_key) DO UPDATE
SET display_name = EXCLUDED.display_name;

UPDATE market_intelligence.provider_tariffs tariff
SET product_id = product.id
FROM market_intelligence.tariff_products product
WHERE product.provider_id = tariff.provider_id
  AND product.plan_key = tariff.plan_key
  AND tariff.product_id IS DISTINCT FROM product.id;

DO $$ BEGIN
    ALTER TABLE market_intelligence.provider_tariffs
        ADD CONSTRAINT provider_tariffs_product_fk
        FOREIGN KEY (product_id) REFERENCES market_intelligence.tariff_products(id)
        ON UPDATE RESTRICT ON DELETE RESTRICT;
EXCEPTION WHEN duplicate_object THEN NULL;
END $$;

CREATE INDEX IF NOT EXISTS provider_tariffs_product_idx
    ON market_intelligence.provider_tariffs (product_id);

ALTER TABLE public.services ADD COLUMN IF NOT EXISTS tariff_product_id bigint;
DO $$ BEGIN
    ALTER TABLE public.services ADD CONSTRAINT services_tariff_product_fk
        FOREIGN KEY (tariff_product_id)
        REFERENCES market_intelligence.tariff_products(id)
        ON UPDATE RESTRICT ON DELETE SET NULL;
EXCEPTION WHEN duplicate_object THEN NULL;
END $$;
CREATE INDEX IF NOT EXISTS services_tariff_product_idx
    ON public.services (tariff_product_id) WHERE tariff_product_id IS NOT NULL;

-- Only exact plan-key/name matches are linked. Unknown or ambiguous service names stay NULL.
UPDATE public.services service
SET tariff_product_id = (
    SELECT product.id
    FROM market_intelligence.tariff_products product
    JOIN market_intelligence.providers provider ON provider.id = product.provider_id
    WHERE provider.code = 'aeza' AND lower(product.plan_key) = lower(service.name)
    LIMIT 1
)
WHERE service.tariff_product_id IS NULL
  AND EXISTS (
      SELECT 1 FROM market_intelligence.tariff_products product
      JOIN market_intelligence.providers provider ON provider.id = product.provider_id
      WHERE provider.code = 'aeza' AND lower(product.plan_key) = lower(service.name)
  );

CREATE OR REPLACE FUNCTION public.match_service_tariff_product() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE should_match boolean;
BEGIN
    IF TG_OP = 'INSERT' THEN
        should_match := NEW.tariff_product_id IS NULL;
    ELSE
        should_match := NEW.name IS DISTINCT FROM OLD.name
            AND NEW.tariff_product_id IS NOT DISTINCT FROM OLD.tariff_product_id;
    END IF;
    IF should_match THEN
        SELECT product.id INTO NEW.tariff_product_id
        FROM market_intelligence.tariff_products product
        JOIN market_intelligence.providers provider ON provider.id = product.provider_id
        WHERE provider.code = 'aeza'
          AND lower(product.plan_key) = lower(NEW.name)
        LIMIT 1;
    END IF;
    RETURN NEW;
END $$;
DROP TRIGGER IF EXISTS services_match_tariff_product ON public.services;
CREATE TRIGGER services_match_tariff_product
    BEFORE INSERT OR UPDATE OF name ON public.services
    FOR EACH ROW EXECUTE FUNCTION public.match_service_tariff_product();

CREATE OR REPLACE VIEW market_intelligence.tariff_prices_rub AS
SELECT p.*, t.provider_id, t.plan_name,
       CASE WHEN p.currency = 'RUB' THEN 1::numeric
            WHEN p.currency IN ('EUR', 'USD') THEN fx.rate ELSE NULL END AS rub_rate,
       CASE WHEN p.currency = 'RUB' THEN p.observed_at::date
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
 AND fx.requested_on = p.observed_at::date;

-- Quality coverage is explicit: unknown specs remain unknown, never inferred as zero.
CREATE OR REPLACE VIEW market_intelligence.tariff_data_quality AS
SELECT provider.code AS provider_code, provider.name AS provider,
       count(DISTINCT product.id) AS products,
       count(DISTINCT tariff.id) AS configurations,
       count(DISTINCT tariff.id) FILTER (WHERE tariff.cpu_cores IS NOT NULL
           AND tariff.ram_mib IS NOT NULL AND tariff.disk_gib IS NOT NULL) AS core_specs_known,
       count(DISTINCT tariff.id) FILTER (WHERE tariff.region IS NOT NULL) AS regions_known,
       count(DISTINCT tariff.id) FILTER (WHERE tariff.disk_type IS NOT NULL) AS disk_type_known,
       count(DISTINCT tariff.id) FILTER (WHERE tariff.attributes ? 'normalized_details') AS terms_normalized,
       count(DISTINCT tariff.id) FILTER (WHERE tariff.attributes #>> '{normalized_details,traffic_terms}' IS NOT NULL) AS traffic_known,
       count(DISTINCT tariff.id) FILTER (WHERE tariff.attributes #>> '{normalized_details,promotion_terms}' IS NOT NULL
           OR tariff.attributes #>> '{normalized_details,restriction_terms}' IS NOT NULL) AS special_conditions_known,
       count(DISTINCT price.id) FILTER (WHERE price.parse_status = 'complete') AS complete_prices,
       count(DISTINCT price.id) FILTER (WHERE price.currency IN ('EUR', 'USD')) AS foreign_currency_prices,
       count(DISTINCT price.id) FILTER (WHERE price.currency IN ('EUR', 'USD')
           AND price.price_rub_minor IS NULL) AS missing_fx_prices
FROM market_intelligence.providers provider
LEFT JOIN market_intelligence.tariff_products product ON product.provider_id = provider.id
LEFT JOIN market_intelligence.provider_tariffs tariff ON tariff.product_id = product.id
LEFT JOIN market_intelligence.tariff_prices_rub price ON price.provider_tariff_id = tariff.id
GROUP BY provider.code, provider.name;

-- Journal only states what the upstream schema actually proves. Account balance
-- adjustments without an explicit source are not mislabeled as refunds.
CREATE OR REPLACE VIEW public.financial_ledger AS
SELECT t.id::text AS event_id, s."ownerId"::bigint AS account_id,
       t."serviceId"::bigint AS service_id, t."createdAt" AS occurred_at,
       CASE t.type WHEN 'order' THEN 'service_order' ELSE 'service_renewal' END AS event_type,
       (-t.amount)::bigint AS amount_cents, t.status::text AS status,
       'transactions'::text AS source
FROM public.transactions t
JOIN public.services s ON s.id = t."serviceId"
WHERE t.status = 'performed'
UNION ALL
SELECT log.id::text, log."entityId"::bigint, NULL::bigint, log."occurredAt",
       CASE WHEN delta > 0 THEN 'topup' ELSE 'replenishment_adjustment' END,
       delta::bigint, 'recorded'::text, 'operation_logs.totalReplenished'::text
FROM (
    SELECT id, "entityId", "occurredAt",
           (after->>'totalReplenished')::numeric - (before->>'totalReplenished')::numeric AS delta
    FROM public.operation_logs
    WHERE entity = 'account' AND action = 'update'
      AND before ? 'totalReplenished' AND after ? 'totalReplenished'
) log
WHERE delta IS NOT NULL AND delta <> 0;

COMMIT;
