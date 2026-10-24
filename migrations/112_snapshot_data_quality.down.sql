DROP VIEW IF EXISTS public.prolong_outcome_quality;

-- Rollback reinstates the legacy function semantics from migration 107.
CREATE OR REPLACE FUNCTION public.client_features(
    as_of timestamptz DEFAULT CURRENT_TIMESTAMP,
    horizon interval DEFAULT '7 days'
) RETURNS TABLE (
    "accountId" bigint, email text, "servicesCount" bigint, "monthlySpend" double precision,
    "shortTermShare" double precision, "noAutoProlongShare" double precision,
    "prolongWaitShare" double precision, "failedProlongShare" double precision,
    "expiringRevenueShare" double precision, "upcomingPayments" bigint,
    "spendableBalance" bigint, "paymentCoverage" double precision
)
LANGUAGE sql STABLE AS $$
    WITH live AS (
        SELECT s."ownerId", s.price, s."paymentTerm", s."autoProlong", s.status, s."expiresAt",
               public.payment_term_seconds(s."paymentTerm") AS term_seconds
        FROM public.services s WHERE s.status <> 'deleted' AND s."createdAt" <= as_of
    ), service_stats AS (
        SELECT "ownerId", count(*) AS services_count,
            sum(price * 2592000 / term_seconds) AS monthly_spend,
            avg(("paymentTerm" IN ('hour', 'week'))::int) AS short_term_share,
            avg((NOT "autoProlong")::int) AS no_auto_prolong_share,
            avg((status = 'prolong_wait')::int) AS prolong_wait_share,
            sum(price * 2592000 / term_seconds)
                FILTER (WHERE "expiresAt" > as_of AND "expiresAt" <= as_of + horizon)
                AS expiring_monthly_spend,
            sum(price * greatest(1, ceil(
                extract(epoch FROM as_of + horizon - greatest("expiresAt", as_of)) / term_seconds
            ))::bigint)
                FILTER (WHERE status IN ('active', 'suspended', 'prolong_wait')
                    AND "expiresAt" <= as_of + horizon) AS upcoming_payments
        FROM live GROUP BY "ownerId"
    ), prolong_stats AS (
        SELECT s."ownerId", avg((t.status = 'not_performed')::int) AS failed_prolong_share
        FROM public.transactions t JOIN public.services s ON s.id = t."serviceId"
        WHERE t.type = 'prolong' AND t."createdAt" <= as_of GROUP BY s."ownerId"
    )
    SELECT a.id::bigint, a.email, coalesce(ss.services_count, 0),
        coalesce(ss.monthly_spend, 0)::float8, ss.short_term_share::float8,
        ss.no_auto_prolong_share::float8, ss.prolong_wait_share::float8,
        ps.failed_prolong_share::float8,
        (coalesce(ss.expiring_monthly_spend, 0) / nullif(ss.monthly_spend, 0))::float8,
        coalesce(ss.upcoming_payments, 0),
        a.balance + CASE WHEN a."bonusState" = 'unlocked' THEN a."bonusBalance" ELSE 0 END,
        CASE WHEN coalesce(ss.upcoming_payments, 0) = 0 THEN 10::float8
             ELSE least((a.balance + CASE WHEN a."bonusState" = 'unlocked' THEN a."bonusBalance" ELSE 0 END)::float8
                        / ss.upcoming_payments, 10::float8) END
    FROM public.accounts a LEFT JOIN service_stats ss ON ss."ownerId" = a.id
    LEFT JOIN prolong_stats ps ON ps."ownerId" = a.id ORDER BY a.id
$$;
