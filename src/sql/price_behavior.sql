WITH ordered_prices AS (
    SELECT product.id AS product_id, product.plan_key, tariff.id AS tariff_id,
           price.observed_at, price.price_minor, price.currency::text AS currency,
           lag(price.price_minor) OVER w AS previous_minor,
           lag(price.currency::text) OVER w AS previous_currency,
           lag(price.observed_at) OVER w AS previous_observed_at
    FROM market_intelligence.tariff_prices price
    JOIN market_intelligence.provider_tariffs tariff ON tariff.id = price.provider_tariff_id
    JOIN market_intelligence.tariff_products product ON product.id = tariff.product_id
    JOIN market_intelligence.providers provider ON provider.id = product.provider_id
    WHERE provider.code = 'aeza' AND price.source_kind = 'live'
      AND price.parse_status = 'complete' AND price.billing_period = 'month'
      AND price.price_kind IN ('regular', 'listed_monthly', 'official_catalog')
    WINDOW w AS (PARTITION BY tariff.id ORDER BY price.observed_at, price.id)
), changes AS (
    SELECT product_id, plan_key, tariff_id, observed_at AS changed_at,
           previous_minor, price_minor AS current_minor, currency,
           previous_observed_at
    FROM ordered_prices
    WHERE previous_minor IS NOT NULL AND previous_currency = currency
      AND previous_minor <> price_minor AND previous_minor > 0
), recent_changes AS (
    SELECT c.*, row_number() OVER (PARTITION BY c.tariff_id ORDER BY c.changed_at DESC) AS rn
    FROM changes c
), study AS (
    SELECT c.*, service.id AS service_id, service."ownerId" AS account_id,
           service."paymentTerm" AS payment_term,
           service.price AS service_price,
           CASE WHEN service.tariff_product_id = c.product_id THEN 'tariff' ELSE 'peer' END AS cohort
    FROM recent_changes c
    JOIN public.services service ON service."ownerId" >= 0
      AND service."createdAt" < c.changed_at - interval '30 days'
      AND (service.tariff_product_id = c.product_id OR
           (service.tariff_product_id <> c.product_id AND c.currency = 'EUR'
            AND service.price BETWEEN c.previous_minor / 2 AND c.previous_minor * 2
            AND EXISTS (
                SELECT 1 FROM public.services target
                WHERE target.tariff_product_id = c.product_id
                  AND target."createdAt" < c.changed_at - interval '30 days'
                  AND target."paymentTerm" = service."paymentTerm"
            )))
    JOIN market_intelligence.tariff_products linked ON linked.id = service.tariff_product_id
    JOIN market_intelligence.providers linked_provider ON linked_provider.id = linked.provider_id
    WHERE c.rn = 1 AND linked_provider.code = 'aeza'
), windows AS (
    SELECT study.*, period.label AS period,
           CASE WHEN period.label = 'before' THEN changed_at - interval '30 days' ELSE changed_at END AS from_at,
           CASE WHEN period.label = 'before' THEN changed_at ELSE changed_at + interval '30 days' END AS to_at
    FROM study CROSS JOIN (VALUES ('before'), ('after')) AS period(label)
), metrics AS (
    SELECT w.product_id, w.plan_key, w.tariff_id, w.changed_at,
           w.previous_observed_at, w.previous_minor, w.current_minor, w.currency,
           w.cohort, w.period,
           count(DISTINCT w.service_id)::bigint AS services,
           count(DISTINCT w.account_id)::bigint AS accounts,
           coalesce(sum(tx.performed_renewals), 0)::bigint AS performed_renewals,
           coalesce(sum(tx.renewal_revenue_minor), 0)::bigint AS renewal_revenue_minor,
           coalesce(sum(status_log.prolong_wait_entries), 0)::bigint AS prolong_wait_entries,
           count(*) FILTER (WHERE status_log.prolong_wait_entries > 0)::bigint AS prolong_wait_services
    FROM windows w
    LEFT JOIN LATERAL (
        SELECT count(*)::bigint AS performed_renewals,
               coalesce(sum(amount), 0)::bigint AS renewal_revenue_minor
        FROM public.transactions tx
        WHERE tx."serviceId" = w.service_id AND tx.type = 'prolong'
          AND tx.status = 'performed' AND tx."createdAt" >= w.from_at
          AND tx."createdAt" < w.to_at
    ) tx ON true
    LEFT JOIN LATERAL (
        SELECT count(*)::bigint AS prolong_wait_entries
        FROM public.operation_logs log
        WHERE log.entity = 'service' AND log."entityId" = w.service_id
          AND log.action = 'update' AND log."occurredAt" >= w.from_at
          AND log."occurredAt" < w.to_at
          AND log.after->>'status' = 'prolong_wait'
          AND log.before->>'status' IS DISTINCT FROM 'prolong_wait'
    ) status_log ON true
    GROUP BY w.product_id, w.plan_key, w.tariff_id, w.changed_at,
             w.previous_observed_at, w.previous_minor, w.current_minor, w.currency,
             w.cohort, w.period
), account_topups AS (
    SELECT w.product_id, w.tariff_id, w.changed_at, w.cohort, w.period,
           coalesce(sum(ledger.amount_cents), 0)::bigint AS account_topups_cents
    FROM (SELECT DISTINCT product_id, tariff_id, changed_at, cohort, period,
                 account_id, from_at, to_at FROM windows) w
    LEFT JOIN public.financial_ledger ledger ON ledger.account_id = w.account_id
      AND ledger.event_type = 'topup'
      AND ledger.occurred_at >= w.from_at AND ledger.occurred_at < w.to_at
    GROUP BY w.product_id, w.tariff_id, w.changed_at, w.cohort, w.period
)
SELECT m.plan_key, m.changed_at, m.previous_observed_at,
       m.previous_minor, m.current_minor, m.currency, m.cohort, m.period,
       m.services, m.accounts, m.performed_renewals,
       m.renewal_revenue_minor, m.prolong_wait_entries, m.prolong_wait_services,
       topups.account_topups_cents,
       (m.changed_at + interval '30 days' <= now()) AS complete_followup,
       (SELECT min("occurredAt") FROM public.operation_logs) <= m.changed_at - interval '30 days'
           AS complete_event_coverage
FROM metrics m
JOIN account_topups topups USING (product_id, tariff_id, changed_at, cohort, period)
ORDER BY m.changed_at DESC, m.plan_key, m.cohort, m.period
LIMIT 400
