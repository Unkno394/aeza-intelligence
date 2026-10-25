ALTER TABLE public.client_settings
    ADD COLUMN version bigint NOT NULL DEFAULT 1,
    ADD COLUMN updated_by text NOT NULL DEFAULT 'system';

ALTER TABLE public.client_snapshots
    ADD COLUMN settings_version bigint NOT NULL DEFAULT 0;

CREATE TABLE public.client_settings_history (
    version bigint PRIMARY KEY,
    settings jsonb NOT NULL,
    updated_at timestamptz NOT NULL,
    updated_by text NOT NULL
);

INSERT INTO public.client_settings_history (version, settings, updated_at, updated_by)
SELECT version, settings, updated_at, updated_by
FROM public.client_settings
ON CONFLICT (version) DO NOTHING;
