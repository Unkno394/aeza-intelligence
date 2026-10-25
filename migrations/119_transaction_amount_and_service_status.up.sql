-- Existing databases keep migration 103 unchanged. Amounts are euro cents.
ALTER TABLE public.transactions DROP CONSTRAINT IF EXISTS transactions_amount_check;
ALTER TABLE public.transactions ADD CONSTRAINT transactions_amount_nonnegative CHECK (amount >= 0);

-- Blocked and deleted services do not contribute recurring revenue or risk exposure.
CREATE OR REPLACE FUNCTION public.client_features(
    as_of timestamptz DEFAULT CURRENT_TIMESTAMP,
    horizon interval DEFAULT '7 days'
) RETURNS TABLE (
    "accountId" bigint, email text, "servicesCount" bigint,
    "monthlySpend" double precision, "shortTermShare" double precision,
    "noAutoProlongShare" double precision, "prolongWaitShare" double precision,
    "failedProlongShare" double precision, "expiringRevenueShare" double precision,
    "upcomingPayments" bigint, "spendableBalance" bigint, "paymentCoverage" double precision
)
LANGUAGE sql STABLE AS $$
    WITH live AS (
        SELECT s."ownerId", s.price, s."paymentTerm", s."autoProlong", s.status, s."expiresAt",
               public.payment_term_seconds(s."paymentTerm") AS term_seconds
        FROM public.services s
        WHERE s.status IN ('active', 'suspended', 'prolong_wait') AND s."createdAt" <= as_of
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
                FILTER (WHERE "expiresAt" <= as_of + horizon) AS upcoming_payments
        FROM live GROUP BY "ownerId"
    )
    SELECT a.id::bigint, a.email,
        coalesce(ss.services_count, 0), coalesce(ss.monthly_spend, 0)::float8,
        ss.short_term_share::float8, ss.no_auto_prolong_share::float8,
        ss.prolong_wait_share::float8,
        NULL::float8 AS "failedProlongShare",
        (coalesce(ss.expiring_monthly_spend, 0) / nullif(ss.monthly_spend, 0))::float8,
        coalesce(ss.upcoming_payments, 0),
        a.balance + CASE WHEN a."bonusState" = 'unlocked' THEN a."bonusBalance" ELSE 0 END,
        CASE WHEN coalesce(ss.services_count, 0) = 0 OR coalesce(ss.upcoming_payments, 0) = 0
             THEN NULL
             ELSE least(
                (a.balance + CASE WHEN a."bonusState" = 'unlocked' THEN a."bonusBalance" ELSE 0 END)::float8
                    / ss.upcoming_payments, 10.0)
        END
    FROM public.accounts a LEFT JOIN service_stats ss ON ss."ownerId" = a.id
    ORDER BY a.id
$$;
