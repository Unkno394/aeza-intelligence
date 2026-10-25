-- Browser sessions created by market_api after an OIDC login. Only a SHA-256
-- hash of the cookie token is stored, so a database read does not reveal live
-- sessions.
CREATE TABLE public.admin_sessions (
    token_hash bytea PRIMARY KEY,
    admin_id bigint NOT NULL REFERENCES public.admins(id),
    created_at timestamptz NOT NULL DEFAULT now(),
    expires_at timestamptz NOT NULL
);

CREATE INDEX admin_sessions_expires_idx ON public.admin_sessions (expires_at);
CREATE INDEX admin_sessions_admin_idx ON public.admin_sessions (admin_id);
