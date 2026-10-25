CREATE INDEX IF NOT EXISTS services_price_control_idx
    ON public.services (name, "paymentTerm", "expiresAt")
    INCLUDE ("ownerId", "createdAt");

CREATE INDEX IF NOT EXISTS transactions_price_window_idx
    ON public.transactions ("serviceId", "performedAt")
    INCLUDE (amount, type)
    WHERE status = 'performed';
