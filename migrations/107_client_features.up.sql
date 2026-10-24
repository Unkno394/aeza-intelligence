-- Per-client risk feature vector, computed as of a moment in time.
--
-- "Live" services are the ones not deleted and already created at as_of. Prices are per
-- payment term, so revenue is normalized to a 30-day run rate ("monthlySpend").

-- Payment term length; months are approximated as 30 days.
CREATE FUNCTION public.payment_term_seconds(term text) RETURNS double precision
LANGUAGE sql IMMUTABLE STRICT AS $$
    SELECT CASE term
        WHEN 'hour' THEN 3600
        WHEN 'week' THEN 7 * 86400
        WHEN 'month' THEN 30 * 86400
        WHEN 'quarter_year' THEN 91 * 86400
        WHEN 'half_year' THEN 182 * 86400
        WHEN 'year' THEN 365 * 86400
    END::double precision
$$;

CREATE FUNCTION public.client_features(
    as_of timestamptz DEFAULT CURRENT_TIMESTAMP,
    horizon interval DEFAULT '7 days'
) RETURNS TABLE (
    "accountId" bigint,
    email text,
    -- Context: what kind of client this is.
    "servicesCount" bigint,
    "monthlySpend" double precision,
    "shortTermShare" double precision,
    -- Risk features. Shares are NULL for clients without live services,
    -- "failedProlongShare" is NULL without prolong attempts.
    "noAutoProlongShare" double precision,
    "prolongWaitShare" double precision,
    "failedProlongShare" double precision,
    -- Share of monthly revenue from services expiring within the horizon.
    "expiringRevenueShare" double precision,
    -- Payments needed within the horizon to keep active, suspended and prolong_wait
    -- services running (hourly services renew many times), in euro cents.
    "upcomingPayments" bigint,
    -- balance plus bonusBalance when bonuses are unlocked; withdrawBalance is referral
    -- money and doesn't count.
    "spendableBalance" bigint,
    -- spendableBalance / upcomingPayments, capped at 10 (10 also when nothing is due).
    "paymentCoverage" double precision
)
LANGUAGE sql STABLE AS $$
    WITH live AS (
        SELECT s."ownerId", s.price, s."paymentTerm", s."autoProlong", s.status, s."expiresAt",
               public.payment_term_seconds(s."paymentTerm") AS term_seconds
        FROM public.services s
        WHERE s.status <> 'deleted' AND s."createdAt" <= as_of
    ),
    service_stats AS (
        SELECT "ownerId",
            count(*) AS services_count,
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
                          AND "expiresAt" <= as_of + horizon)
                AS upcoming_payments
        FROM live
        GROUP BY "ownerId"
    ),
    prolong_stats AS (
        SELECT s."ownerId", avg((t.status = 'not_performed')::int) AS failed_prolong_share
        FROM public.transactions t
        JOIN public.services s ON s.id = t."serviceId"
        WHERE t.type = 'prolong' AND t."createdAt" <= as_of
        GROUP BY s."ownerId"
    )
    SELECT
        a.id::bigint,
        a.email,
        coalesce(ss.services_count, 0),
        coalesce(ss.monthly_spend, 0)::float8,
        ss.short_term_share::float8,
        ss.no_auto_prolong_share::float8,
        ss.prolong_wait_share::float8,
        ps.failed_prolong_share::float8,
        (coalesce(ss.expiring_monthly_spend, 0) / nullif(ss.monthly_spend, 0))::float8,
        coalesce(ss.upcoming_payments, 0),
        b.spendable,
        CASE WHEN coalesce(ss.upcoming_payments, 0) = 0 THEN 10
             ELSE least(b.spendable::float8 / ss.upcoming_payments, 10)
        END
    FROM public.accounts a
    CROSS JOIN LATERAL (
        SELECT a.balance + CASE WHEN a."bonusState" = 'unlocked' THEN a."bonusBalance" ELSE 0 END
            AS spendable
    ) b
    LEFT JOIN service_stats ss ON ss."ownerId" = a.id
    LEFT JOIN prolong_stats ps ON ps."ownerId" = a.id
    ORDER BY a.id
$$;
