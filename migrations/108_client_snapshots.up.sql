CREATE TABLE public.client_snapshots (
    account_id bigint NOT NULL,
    observed_on date NOT NULL,
    observed_at timestamptz NOT NULL DEFAULT now(),
    features jsonb NOT NULL,
    assessment jsonb NOT NULL,
    model_state jsonb NOT NULL,
    PRIMARY KEY (account_id, observed_on)
);

CREATE INDEX client_snapshots_recent_idx
    ON public.client_snapshots (account_id, observed_on DESC);
