-- Amounts are euro cents. Migration 103 allowed only 0–1000, which rejects
-- real snapshots uploaded through PUT /api/v1/source/snapshot.
ALTER TABLE public.transactions DROP CONSTRAINT IF EXISTS transactions_amount_check;
ALTER TABLE public.transactions ADD CONSTRAINT transactions_amount_nonnegative CHECK (amount >= 0);
