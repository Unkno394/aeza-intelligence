-- Isolated negative-ID profiles for client_features edge checks.
-- Apply only to a disposable, migrated test database.
BEGIN;
DO $$ BEGIN
  IF EXISTS (SELECT 1 FROM public.accounts WHERE id BETWEEN -9800004 AND -9800001)
  THEN RAISE EXCEPTION 'Edge-case fixture IDs are occupied'; END IF;
END $$;

INSERT INTO public.accounts (id, email, balance)
VALUES
  (-9800001, 'edge-empty@example.test', 0),
  (-9800002, 'edge-blocked@example.test', 0),
  (-9800003, 'edge-no-payments@example.test', 0),
  (-9800004, 'edge-no-upcoming@example.test', 5000);

INSERT INTO public.services
  (id, name, "ownerId", "paymentTerm", "autoProlong", "createdAt", "expiresAt", status, price)
VALUES
  (-9810002, 'EDGE-BLOCKED', -9800002, 'month', false,
   now() - interval '90 days', now() + interval '1 day', 'blocked', 500),
  (-9810003, 'EDGE-NO-PAYMENTS', -9800003, 'month', true,
   now() - interval '90 days', now() + interval '1 day', 'active', 500),
  (-9810004, 'EDGE-NO-UPCOMING', -9800004, 'year', true,
   now() - interval '90 days', now() + interval '200 days', 'active', 500);
COMMIT;

-- Controlled billing-period and spendable-balance fixture. The large withdrawBalance
-- is referral money and must not contribute to payment coverage.
BEGIN;
DO $$ BEGIN
  IF EXISTS (SELECT 1 FROM public.accounts WHERE id IN (-9800005, -9800006))
     OR EXISTS (SELECT 1 FROM public.services WHERE id BETWEEN -9810014 AND -9810005)
  THEN RAISE EXCEPTION 'Billing fixture IDs are occupied'; END IF;
END $$;

INSERT INTO public.accounts
  (id, email, balance, "withdrawBalance", "totalReplenished", "bonusBalance", "bonusState")
VALUES
  (-9800005, 'edge-billing-unlocked@example.test', 50, 10000, 40000, 100, 'unlocked'),
  (-9800006, 'edge-billing-locked@example.test', 50, 10000, 40000, 100, 'locked');

INSERT INTO public.services
  (id, name, "ownerId", "paymentTerm", "autoProlong", "createdAt", "expiresAt", status, price)
VALUES
  (-9810005, 'EDGE-hour', -9800005, 'hour', true, '1999-10-03 00:00:00+00', '2000-01-02 00:00:00+00', 'active', 100),
  (-9810006, 'EDGE-week', -9800005, 'week', true, '1999-10-03 00:00:00+00', '2000-01-02 00:00:00+00', 'active', 100),
  (-9810007, 'EDGE-month', -9800005, 'month', true, '1999-10-03 00:00:00+00', '2000-01-02 00:00:00+00', 'active', 100),
  (-9810008, 'EDGE-quarter', -9800005, 'quarter_year', true, '1999-10-03 00:00:00+00', '2000-01-02 00:00:00+00', 'active', 100),
  (-9810009, 'EDGE-halfyear', -9800005, 'half_year', true, '1999-10-03 00:00:00+00', '2000-01-02 00:00:00+00', 'active', 100),
  (-9810010, 'EDGE-year', -9800005, 'year', true, '1999-10-03 00:00:00+00', '2000-01-02 00:00:00+00', 'active', 100),
  (-9810011, 'EDGE-suspended', -9800005, 'month', true, '1999-10-03 00:00:00+00', '2000-01-02 00:00:00+00', 'suspended', 200),
  (-9810012, 'EDGE-waiting', -9800005, 'month', true, '1999-10-03 00:00:00+00', '2000-01-02 00:00:00+00', 'prolong_wait', 300),
  (-9810013, 'EDGE-blocked', -9800005, 'month', true, '1999-10-03 00:00:00+00', '2000-01-02 00:00:00+00', 'blocked', 400),
  (-9810014, 'EDGE-deleted', -9800005, 'month', true, '1999-10-03 00:00:00+00', '2000-01-02 00:00:00+00', 'deleted', 500);
COMMIT;
