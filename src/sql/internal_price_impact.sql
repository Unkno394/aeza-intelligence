-- Real changes to a client's billed service price. Public catalog observations
-- are deliberately excluded: they do not prove what this client paid.
WITH increases AS (
    SELECT log.id AS event_id,
           log."entityId" AS service_id,
           (log.after->>'ownerId')::bigint AS account_id,
           (log.after->>'tariff_product_id')::bigint AS tariff_product_id,
           log.after->>'name' AS plan_name,
           log."occurredAt" AS changed_at,
           (log.before->>'price')::bigint AS previous_price_cents,
           (log.after->>'price')::bigint AS current_price_cents,
           (log.before->>'expiresAt')::timestamptz AS next_renewal_at,
           log.after->>'paymentTerm' AS payment_term,
           (log.before->>'createdAt')::timestamptz <= log."occurredAt" - interval '30 days'
               AS complete_baseline,
           (log.before->>'createdAt')::timestamptz <= log."occurredAt" - interval '60 days'
               AS complete_pretrend
    FROM public.operation_logs log
    WHERE log.entity = 'service' AND log.action = 'update' AND log.outcome = 'success'
      AND log."occurredAt" >= now() - interval '365 days'
      AND log.before ? 'price' AND log.after ? 'price'
      AND (log.before->>'price')::bigint > 0
      AND (log.after->>'price')::bigint > (log.before->>'price')::bigint
      AND log.before->>'name' = log.after->>'name'
      AND log.before->>'paymentTerm' = log.after->>'paymentTerm'
      AND ((log.after->>'ownerId')::bigint > 0
           OR (log.after->>'ownerId')::bigint BETWEEN -9001200 AND -9000001)
)
SELECT increase.event_id::text AS event_id, increase.service_id::text AS service_id,
       increase.account_id::text AS account_id,
       account.email AS account_email, product.plan_key, increase.plan_name, increase.changed_at,
       increase.previous_price_cents, increase.current_price_cents,
       increase.next_renewal_at, increase.payment_term, increase.complete_baseline,
       increase.complete_pretrend,
       increase.changed_at + interval '30 days' <= now() AS complete_followup,
       increase.next_renewal_at >= increase.changed_at
           AND increase.next_renewal_at < increase.changed_at + interval '30 days'
           AS renewal_due_in_window,
       service.status AS current_status, service."autoProlong" AS current_auto_prolong,
       before_tx.renewals AS renewals_before, after_tx.renewals AS renewals_after,
       earlier_tx.renewals AS renewals_earlier,
       coalesce(after_status.prolong_wait, false) AS prolong_wait_after,
       coalesce(after_status.auto_renew_disabled, false) AS auto_renew_disabled_after,
       coalesce(after_status.inactive, false) AS inactive_after,
       (baseline.features->>'services_count')::bigint AS baseline_services_count,
       (followup.features->>'services_count')::bigint AS followup_services_count,
       (baseline.features->>'monthly_spend')::float8 AS baseline_monthly_spend,
       (followup.features->>'monthly_spend')::float8 AS followup_monthly_spend,
       before_spend.amount AS account_spend_before_cents,
       earlier_spend.amount AS account_spend_earlier_cents,
       after_spend.amount AS account_spend_after_cents,
       control.account_id::text AS control_account_id,
       control.services_before AS control_services_before,
       control.services_after AS control_services_after,
       control.spend_before_cents AS control_spend_before_cents,
       control.spend_after_cents AS control_spend_after_cents,
       control.renewed_after AS control_renewed_after,
       control.renewals_before AS control_renewals_before,
       control.renewals_earlier AS control_renewals_earlier,
       control.spend_earlier_cents AS control_spend_earlier_cents
FROM increases increase
LEFT JOIN public.accounts account ON account.id = increase.account_id
LEFT JOIN market_intelligence.tariff_products product ON product.id = increase.tariff_product_id
LEFT JOIN public.services service ON service.id = increase.service_id
LEFT JOIN LATERAL (
    SELECT count(*)::bigint AS renewals
    FROM public.transactions tx
    WHERE tx."serviceId" = increase.service_id AND tx.type = 'prolong'
      AND tx.status = 'performed'
      AND tx."performedAt" >= increase.changed_at - interval '30 days'
      AND tx."performedAt" < increase.changed_at
) before_tx ON true
LEFT JOIN LATERAL (
    SELECT count(*)::bigint AS renewals FROM public.transactions tx
    WHERE tx."serviceId" = increase.service_id AND tx.type = 'prolong'
      AND tx.status = 'performed'
      AND tx."performedAt" >= increase.changed_at - interval '60 days'
      AND tx."performedAt" < increase.changed_at - interval '30 days'
) earlier_tx ON true
LEFT JOIN LATERAL (
    SELECT count(*)::bigint AS renewals
    FROM public.transactions tx
    WHERE tx."serviceId" = increase.service_id AND tx.type = 'prolong'
      AND tx.status = 'performed'
      AND tx."performedAt" >= increase.changed_at
      AND tx."performedAt" < increase.changed_at + interval '30 days'
) after_tx ON true
LEFT JOIN LATERAL (
    SELECT bool_or(log.after->>'status' = 'prolong_wait'
                   AND log.before->>'status' IS DISTINCT FROM 'prolong_wait') AS prolong_wait,
           bool_or(log.before->>'autoProlong' = 'true'
                   AND log.after->>'autoProlong' = 'false') AS auto_renew_disabled,
           bool_or(log.after->>'status' IN ('deleted', 'suspended', 'blocked')
                   AND log.before->>'status' IS DISTINCT FROM log.after->>'status') AS inactive
    FROM public.operation_logs log
    WHERE log.entity = 'service' AND log."entityId" = increase.service_id
      AND log.action = 'update' AND log.outcome = 'success'
      AND log.id > increase.event_id
      AND log."occurredAt" >= increase.changed_at
      AND log."occurredAt" < increase.changed_at + interval '30 days'
) after_status ON true
LEFT JOIN LATERAL (
    SELECT snapshot.features
    FROM public.client_snapshots snapshot
    WHERE snapshot.account_id = increase.account_id
      AND snapshot.observed_on BETWEEN (increase.changed_at AT TIME ZONE 'UTC')::date - 3
                                   AND (increase.changed_at AT TIME ZONE 'UTC')::date - 1
    ORDER BY snapshot.observed_on DESC LIMIT 1
) baseline ON true
LEFT JOIN LATERAL (
    SELECT snapshot.features
    FROM public.client_snapshots snapshot
    WHERE snapshot.account_id = increase.account_id
      AND snapshot.observed_on BETWEEN (increase.changed_at AT TIME ZONE 'UTC')::date + 30
                                   AND (increase.changed_at AT TIME ZONE 'UTC')::date + 37
    ORDER BY snapshot.observed_on ASC LIMIT 1
) followup ON true
LEFT JOIN LATERAL (
    SELECT coalesce(sum(tx.amount), 0)::bigint AS amount
    FROM public.transactions tx
    JOIN public.services paid_service ON paid_service.id = tx."serviceId"
    WHERE paid_service."ownerId" = increase.account_id
      AND tx.status = 'performed' AND tx.type IN ('order', 'prolong')
      AND tx."performedAt" >= increase.changed_at - interval '30 days'
      AND tx."performedAt" < increase.changed_at
) before_spend ON true
LEFT JOIN LATERAL (
    SELECT coalesce(sum(tx.amount), 0)::bigint AS amount
    FROM public.transactions tx JOIN public.services paid_service ON paid_service.id = tx."serviceId"
    WHERE paid_service."ownerId" = increase.account_id
      AND tx.status = 'performed' AND tx.type IN ('order', 'prolong')
      AND tx."performedAt" >= increase.changed_at - interval '60 days'
      AND tx."performedAt" < increase.changed_at - interval '30 days'
) earlier_spend ON true
LEFT JOIN LATERAL (
    SELECT coalesce(sum(tx.amount), 0)::bigint AS amount
    FROM public.transactions tx
    JOIN public.services paid_service ON paid_service.id = tx."serviceId"
    WHERE paid_service."ownerId" = increase.account_id
      AND tx.status = 'performed' AND tx.type IN ('order', 'prolong')
      AND tx."performedAt" >= increase.changed_at
      AND tx."performedAt" < increase.changed_at + interval '30 days'
) after_spend ON true
LEFT JOIN LATERAL (
    SELECT peer."ownerId" AS account_id,
           (peer_before.features->>'services_count')::bigint AS services_before,
           (peer_after.features->>'services_count')::bigint AS services_after,
           peer_spend_before.amount AS spend_before_cents,
           peer_spend_earlier.amount AS spend_earlier_cents,
           peer_spend_after.amount AS spend_after_cents,
           peer_renewals_before.count AS renewals_before,
           peer_renewals_earlier.count AS renewals_earlier,
           EXISTS (
             SELECT 1 FROM public.transactions peer_tx
             WHERE peer_tx."serviceId" = peer.id AND peer_tx.type = 'prolong'
               AND peer_tx.status = 'performed'
               AND peer_tx."performedAt" >= increase.changed_at
               AND peer_tx."performedAt" < increase.changed_at + interval '30 days'
           ) AS renewed_after
    FROM public.services peer
    JOIN public.accounts peer_account ON peer_account.id = peer."ownerId"
    JOIN LATERAL (
        SELECT (log.after->>'expiresAt')::timestamptz AS next_renewal_at
        FROM public.operation_logs log
        WHERE log.entity = 'service' AND log."entityId" = peer.id
          AND log.outcome = 'success' AND log.after ? 'expiresAt'
          AND log."occurredAt" <= increase.changed_at
        ORDER BY log."occurredAt" DESC, log.id DESC LIMIT 1
    ) peer_state ON true
    JOIN LATERAL (
        SELECT features FROM public.client_snapshots s
        WHERE s.account_id = peer."ownerId"
          AND s.observed_on BETWEEN (increase.changed_at AT TIME ZONE 'UTC')::date - 3
                                AND (increase.changed_at AT TIME ZONE 'UTC')::date - 1
        ORDER BY s.observed_on DESC LIMIT 1
    ) peer_before ON true
    JOIN LATERAL (
        SELECT features FROM public.client_snapshots s
        WHERE s.account_id = peer."ownerId"
          AND s.observed_on BETWEEN (increase.changed_at AT TIME ZONE 'UTC')::date + 30
                                AND (increase.changed_at AT TIME ZONE 'UTC')::date + 37
        ORDER BY s.observed_on ASC LIMIT 1
    ) peer_after ON true
    CROSS JOIN LATERAL (
        SELECT coalesce(sum(t.amount), 0)::bigint AS amount
        FROM public.transactions t JOIN public.services paid ON paid.id = t."serviceId"
        WHERE paid."ownerId" = peer."ownerId" AND t.status = 'performed'
          AND t.type IN ('order', 'prolong')
          AND t."performedAt" >= increase.changed_at - interval '30 days'
          AND t."performedAt" < increase.changed_at
    ) peer_spend_before
    CROSS JOIN LATERAL (
        SELECT coalesce(sum(t.amount), 0)::bigint AS amount
        FROM public.transactions t JOIN public.services paid ON paid.id = t."serviceId"
        WHERE paid."ownerId" = peer."ownerId" AND t.status = 'performed'
          AND t.type IN ('order', 'prolong')
          AND t."performedAt" >= increase.changed_at - interval '60 days'
          AND t."performedAt" < increase.changed_at - interval '30 days'
    ) peer_spend_earlier
    CROSS JOIN LATERAL (
        SELECT count(*)::bigint AS count FROM public.transactions t
        WHERE t."serviceId" = peer.id AND t.type = 'prolong' AND t.status = 'performed'
          AND t."performedAt" >= increase.changed_at - interval '30 days'
          AND t."performedAt" < increase.changed_at
    ) peer_renewals_before
    CROSS JOIN LATERAL (
        SELECT count(*)::bigint AS count FROM public.transactions t
        WHERE t."serviceId" = peer.id AND t.type = 'prolong' AND t.status = 'performed'
          AND t."performedAt" >= increase.changed_at - interval '60 days'
          AND t."performedAt" < increase.changed_at - interval '30 days'
    ) peer_renewals_earlier
    CROSS JOIN LATERAL (
        SELECT coalesce(sum(t.amount), 0)::bigint AS amount
        FROM public.transactions t JOIN public.services paid ON paid.id = t."serviceId"
        WHERE paid."ownerId" = peer."ownerId" AND t.status = 'performed'
          AND t.type IN ('order', 'prolong')
          AND t."performedAt" >= increase.changed_at
          AND t."performedAt" < increase.changed_at + interval '30 days'
    ) peer_spend_after
    WHERE peer."ownerId" <> increase.account_id
      AND (increase.account_id > 0 OR peer."ownerId" = increase.account_id - 600)
      AND ((increase.account_id > 0 AND peer."ownerId" > 0
            AND peer_account.email NOT LIKE '%@example.test')
           OR (increase.account_id < 0 AND peer."ownerId" BETWEEN -9001200 AND -9000001
               AND peer_account.email LIKE 'price-demo-%@example.test'))
      AND peer.name = increase.plan_name AND peer."paymentTerm"::text = increase.payment_term
      AND peer."createdAt" <= increase.changed_at - interval '30 days'
      AND peer_state.next_renewal_at >= increase.changed_at
      AND peer_state.next_renewal_at < increase.changed_at + interval '30 days'
      AND NOT EXISTS (
        SELECT 1 FROM public.operation_logs change
        WHERE change.entity = 'service' AND change."entityId" = peer.id
          AND change.action = 'update' AND change.outcome = 'success'
          AND change."occurredAt" >= increase.changed_at - interval '30 days'
          AND change."occurredAt" < increase.changed_at + interval '30 days'
          AND change.before ? 'price' AND change.after ? 'price'
          AND change.before->>'price' IS DISTINCT FROM change.after->>'price'
      )
      AND (baseline.features->>'monthly_spend')::float8 > 0
      AND (peer_before.features->>'monthly_spend')::float8 BETWEEN
          (baseline.features->>'monthly_spend')::float8 * 0.7 AND
          (baseline.features->>'monthly_spend')::float8 * 1.3
      AND abs((peer_before.features->>'services_count')::bigint
              - (baseline.features->>'services_count')::bigint) <= 1
      AND peer_renewals_before.count = before_tx.renewals
      AND abs((before_spend.amount - earlier_spend.amount)
              - (peer_spend_before.amount - peer_spend_earlier.amount))
          <= greatest(before_spend.amount, peer_spend_before.amount, 1) * 0.2
    ORDER BY abs((peer_before.features->>'monthly_spend')::float8 -
                 (baseline.features->>'monthly_spend')::float8),
             CASE WHEN increase.account_id < 0
                  THEN abs(peer."ownerId" - (increase.account_id - 600)) ELSE 0 END,
             peer.id
    LIMIT 1
) control ON increase.changed_at + interval '30 days' <= now()
ORDER BY increase.changed_at DESC, increase.event_id DESC
