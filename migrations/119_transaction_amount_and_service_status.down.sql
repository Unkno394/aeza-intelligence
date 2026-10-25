ALTER TABLE public.transactions DROP CONSTRAINT IF EXISTS transactions_amount_nonnegative;
ALTER TABLE public.transactions ADD CONSTRAINT transactions_amount_check CHECK (amount BETWEEN 0 AND 1000);
