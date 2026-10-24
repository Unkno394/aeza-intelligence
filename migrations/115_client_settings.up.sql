-- Risk Score settings shared by every frontend session. The single row is
-- written by market_api when a client sends updateSettings over /api/client/ws.
CREATE TABLE public.client_settings (
    id boolean PRIMARY KEY DEFAULT true CHECK (id),
    settings jsonb NOT NULL,
    updated_at timestamptz NOT NULL DEFAULT now()
);
