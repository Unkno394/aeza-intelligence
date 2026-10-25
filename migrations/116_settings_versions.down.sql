ALTER TABLE public.client_snapshots
    DROP COLUMN settings_version;

DROP TABLE public.client_settings_history;

ALTER TABLE public.client_settings
    DROP COLUMN updated_by,
    DROP COLUMN version;
