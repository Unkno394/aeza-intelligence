-- Scenario observations are linked to real IDs but never change billed services.
CREATE TABLE public.price_demo_events (
    event_id text PRIMARY KEY,
    account_id bigint NOT NULL REFERENCES public.accounts(id),
    service_id bigint NOT NULL REFERENCES public.services(id),
    control_account_id bigint NOT NULL REFERENCES public.accounts(id),
    changed_at timestamptz NOT NULL,
    event jsonb NOT NULL
);
CREATE INDEX price_demo_events_account_idx ON public.price_demo_events (account_id, changed_at DESC);
