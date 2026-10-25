-- Idempotent, isolated scenario history for up to 720 existing fixture customers and
-- 600 unchanged controls. No account, service, transaction or live log is edited.
WITH account_stats AS (
    SELECT s."ownerId" AS account_id,
           count(*)::bigint AS services_count,
           sum(CASE s."paymentTerm"
               WHEN 'hour' THEN s.price * 24 * 30
               WHEN 'week' THEN s.price * 4
               WHEN 'quarter_year' THEN s.price / 3
               WHEN 'half_year' THEN s.price / 6
               WHEN 'year' THEN s.price / 12
               ELSE s.price END)::bigint AS monthly_spend
    FROM public.services s
    WHERE s.status IN ('active', 'suspended') AND s.price > 0
    GROUP BY s."ownerId"
), candidates AS (
    SELECT a.id AS account_id, a.email, s.id AS service_id, s.name AS plan_name,
           s.price AS old_price, s."paymentTerm" AS payment_term,
           stats.services_count, stats.monthly_spend,
           row_number() OVER (ORDER BY md5(a.id::text || ':price-scenario-v1')) AS rn
    FROM public.accounts a
    JOIN account_stats stats ON stats.account_id = a.id
    JOIN LATERAL (
        SELECT service.id, service.name, service.price, service."paymentTerm"
        FROM public.services service
        WHERE service."ownerId" = a.id
          AND service.status IN ('active', 'suspended') AND service.price > 0
        ORDER BY service.price DESC, service.id LIMIT 1
    ) s ON true
    WHERE a.email LIKE '%@example.test'
), paired AS (
    SELECT t.*, c.account_id AS control_id, c.services_count AS control_services,
           c.monthly_spend AS control_spend,
           ((t.rn - 1) / 120)::integer % 3 AS band,
           ((t.rn - 1) / 360)::integer AS period,
           ((t.rn - 1) % 120)::integer AS slot,
           CASE ((t.rn - 1) / 120)::integer % 3
               WHEN 0 THEN 1.05 WHEN 1 THEN 1.18 ELSE 1.35 END AS multiplier
    FROM candidates t
    JOIN candidates c ON c.rn = 721 + ((t.rn - 1) % 600)
    WHERE t.rn <= 720
), observations AS (
    SELECT p.*,
           (date_trunc('day', now()) - CASE WHEN p.period = 0 THEN interval '270 days' ELSE interval '75 days' END) AS changed_at,
           p.slot < CASE p.band WHEN 0 THEN 108 WHEN 1 THEN 90 ELSE 66 END AS renewed,
           p.slot < 102 AS control_renewed,
           round(p.old_price * p.multiplier)::bigint AS new_price
    FROM paired p
)
INSERT INTO public.price_demo_events
    (event_id, account_id, service_id, control_account_id, changed_at, event)
SELECT 'price-scenario-v1-' || o.account_id, o.account_id, o.service_id, o.control_id,
       o.changed_at,
       jsonb_build_object(
           'eventId', 'price-scenario-v1-' || o.account_id,
           'serviceId', o.service_id::text,
           'accountId', o.account_id::text,
           'accountEmail', o.email,
           'planKey', null,
           'planName', o.plan_name,
           'changedAt', o.changed_at,
           'previousPriceCents', o.old_price,
           'currentPriceCents', o.new_price,
           'nextRenewalAt', o.changed_at + interval '15 days',
           'paymentTerm', o.payment_term,
           'completeBaseline', true,
           'completePretrend', true,
           'completeFollowup', true,
           'renewalDueInWindow', true,
           'currentStatus', CASE WHEN o.renewed THEN 'active' ELSE 'prolong_wait' END,
           'currentAutoProlong', o.renewed,
           'renewalsEarlier', 1,
           'renewalsBefore', 1,
           'renewalsAfter', CASE WHEN o.renewed THEN 1 ELSE 0 END,
           'prolongWaitAfter', NOT o.renewed,
           'autoRenewDisabledAfter', NOT o.renewed,
           'inactiveAfter', false,
           'baselineServicesCount', o.services_count,
           'followupServicesCount', greatest(0, o.services_count - CASE WHEN o.renewed THEN 0 ELSE 1 END),
           'baselineMonthlySpend', o.monthly_spend,
           'followupMonthlySpend', CASE WHEN o.renewed THEN o.monthly_spend + o.new_price - o.old_price
                                            ELSE greatest(0, o.monthly_spend - o.old_price) END,
           'accountSpendEarlierCents', o.monthly_spend,
           'accountSpendBeforeCents', o.monthly_spend,
           'accountSpendAfterCents', CASE WHEN o.renewed THEN o.monthly_spend + o.new_price - o.old_price
                                          ELSE greatest(0, o.monthly_spend - o.old_price) END,
           'controlAccountId', o.control_id::text,
           'controlServicesBefore', o.control_services,
           'controlServicesAfter', greatest(0, o.control_services - CASE WHEN o.control_renewed THEN 0 ELSE 1 END),
           'controlSpendEarlierCents', o.control_spend,
           'controlSpendBeforeCents', o.control_spend,
           'controlSpendAfterCents', CASE WHEN o.control_renewed THEN o.control_spend ELSE greatest(0, o.control_spend * 0.8)::bigint END,
           'controlRenewedAfter', o.control_renewed,
           'controlRenewalsEarlier', 1,
           'controlRenewalsBefore', 1,
           'dataMode', 'synthetic'
       )
FROM observations o
ON CONFLICT (event_id) DO NOTHING;
