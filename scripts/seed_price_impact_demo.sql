-- 1,200 isolated demo accounts: 600 price increases and 600 unchanged controls.
-- Run with psql -v ON_ERROR_STOP=1 -f scripts/seed_price_impact_demo.sql on a migrated
-- development database. This is additive and refuses to overwrite existing rows.
BEGIN;
DO $$ BEGIN
  IF EXISTS (SELECT 1 FROM public.accounts WHERE id BETWEEN -9001200 AND -9000001)
     OR EXISTS (SELECT 1 FROM public.services WHERE id BETWEEN -9101200 AND -9100001)
  THEN RAISE EXCEPTION 'Price demo IDs already exist; refusing to overwrite'; END IF;
END $$;

CREATE TEMP TABLE price_demo_rows ON COMMIT DROP AS
SELECT n, n <= 600 AS treated,
       CASE WHEN n <= 600 THEN n ELSE n - 600 END AS pair,
       -9000000 - n::bigint AS account_id,
       -9100000 - n::bigint AS service_id,
       CASE WHEN n <= 600 THEN (n - 1) / 300 ELSE (n - 601) / 300 END AS period,
       CASE WHEN n <= 600 THEN ((n - 1) % 300) / 100
            ELSE ((n - 601) % 300) / 100 END AS band
FROM generate_series(1, 1200) AS n;

ALTER TABLE price_demo_rows ADD COLUMN changed_at timestamptz;
UPDATE price_demo_rows SET changed_at =
  (now() AT TIME ZONE 'UTC')::date - CASE WHEN period = 0 THEN 270 ELSE 75 END
  + interval '12 hours';

INSERT INTO public.accounts (id, email, balance, "totalReplenished")
SELECT account_id, format('price-demo-%s@example.test', n), 5000, 5000
FROM price_demo_rows;

INSERT INTO public.services
  (id, name, "ownerId", "paymentTerm", "autoProlong", "createdAt", "expiresAt", status, price)
SELECT service_id, 'PRICE-DEMO-1', account_id, 'month', true,
       changed_at - interval '100 days', changed_at + interval '15 days', 'active',
       CASE WHEN treated THEN CASE band WHEN 0 THEN 525 WHEN 1 THEN 575 ELSE 675 END
            ELSE 500 END
FROM price_demo_rows;

-- Historical service state makes the control's renewal due date observable
-- at the treated client's change date.
INSERT INTO public.operation_logs
  ("occurredAt", "actorType", action, entity, "entityId", before, after, outcome, metadata)
SELECT changed_at - interval '1 day', 'system', 'update', 'service', service_id,
       jsonb_build_object('expiresAt', changed_at + interval '15 days'),
       jsonb_build_object('expiresAt', changed_at + interval '15 days'),
       'success', '{"synthetic_price_demo":true}'::jsonb
FROM price_demo_rows WHERE NOT treated;

-- The logged before/after state is what the production query uses for actual
-- billed price changes. No real account or tariff product is modified.
INSERT INTO public.operation_logs
  ("occurredAt", "actorType", action, entity, "entityId", before, after, outcome, metadata)
SELECT changed_at, 'system', 'update', 'service', service_id,
       jsonb_build_object('ownerId', account_id, 'name', 'PRICE-DEMO-1',
         'paymentTerm', 'month', 'createdAt', changed_at - interval '100 days',
         'expiresAt', changed_at + interval '15 days', 'price', 500),
       jsonb_build_object('ownerId', account_id, 'name', 'PRICE-DEMO-1',
         'paymentTerm', 'month', 'createdAt', changed_at - interval '100 days',
         'expiresAt', changed_at + interval '15 days', 'price',
         CASE band WHEN 0 THEN 525 WHEN 1 THEN 575 ELSE 675 END),
       'success', '{"synthetic_price_demo":true}'::jsonb
FROM price_demo_rows WHERE treated ORDER BY changed_at, service_id;

-- Each account has one performed payment before the event. Subsequent payment
-- and renewal success vary by band in both periods; controls remain stable.
INSERT INTO public.transactions
  (id, amount, status, "performedAt", "createdAt", type, "serviceId")
SELECT -9400000 - n::bigint, 500, 'performed', changed_at - interval '45 days',
       changed_at - interval '45 days 1 minute', 'prolong', service_id
FROM price_demo_rows;
INSERT INTO public.transactions
  (id, amount, status, "performedAt", "createdAt", type, "serviceId")
SELECT -9200000 - n::bigint, 500, 'performed', changed_at - interval '15 days',
       changed_at - interval '15 days 1 minute', 'prolong', service_id
FROM price_demo_rows;
INSERT INTO public.transactions
  (id, amount, status, "performedAt", "createdAt", type, "serviceId")
SELECT -9300000 - n::bigint, CASE WHEN treated THEN 500 + band * 50 ELSE 500 END,
       'performed', changed_at + interval '15 days',
       changed_at + interval '14 days 23 hours', 'prolong', service_id
FROM price_demo_rows
WHERE CASE WHEN treated THEN (pair - 1) % 100 < 78 - band * 10
           ELSE (pair - 1) % 100 < 80 END;

-- Unrenewed treated services have an explicit adverse state, so their outcome
-- is known rather than guessed from an absent transaction.
INSERT INTO public.operation_logs
  ("occurredAt", "actorType", action, entity, "entityId", before, after, outcome, metadata)
SELECT changed_at + interval '16 days', 'system', 'update', 'service', service_id,
       jsonb_build_object('status', 'active', 'autoProlong', true),
       jsonb_build_object('status', 'prolong_wait', 'autoProlong', false),
       'success', '{"synthetic_price_demo":true}'::jsonb
FROM price_demo_rows
WHERE treated AND (pair - 1) % 100 >= 78 - band * 10;

INSERT INTO public.client_snapshots
  (account_id, observed_on, observed_at, features, assessment, model_state, settings_version)
SELECT account_id, (changed_at AT TIME ZONE 'UTC')::date - 1,
       changed_at - interval '1 day',
       jsonb_build_object('services_count', 1, 'monthly_spend',
                          CASE WHEN pair % 2 = 0 THEN 400 ELSE 600 END),
       '{"synthetic_price_demo":true}'::jsonb, '{}'::jsonb, 0
FROM price_demo_rows;
INSERT INTO public.client_snapshots
  (account_id, observed_on, observed_at, features, assessment, model_state, settings_version)
SELECT account_id, (changed_at AT TIME ZONE 'UTC')::date + 31,
       changed_at + interval '31 days',
       jsonb_build_object('services_count', CASE WHEN treated AND (pair - 1) % 100 >= 78 - band * 10 THEN 0 ELSE 1 END,
                          'monthly_spend', CASE WHEN pair % 2 = 0 THEN 400 ELSE 600 END
                            + CASE WHEN treated THEN band * 50 ELSE 0 END),
       '{"synthetic_price_demo":true}'::jsonb, '{}'::jsonb, 0
FROM price_demo_rows;

COMMIT;
