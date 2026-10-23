-- Append-only log of every operation on accounts, services and transactions.
-- Row changes are written by the triggers from 006_operation_logs_triggers.up.sql;
-- the application adds bulk_load summaries and failures (which a trigger can't record,
-- since a failed transaction rolls back its own log rows).
--
-- "entityId" and "accountId" have no foreign keys on purpose: log entries must outlive
-- the rows they describe.

CREATE TABLE public.operation_logs (
    id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    "occurredAt" timestamptz(3) NOT NULL DEFAULT CURRENT_TIMESTAMP,
    -- admin: through the admin panel; system: cron, auto-prolong, loader; user: account owner.
    "actorType" text NOT NULL CHECK ("actorType" IN ('admin', 'system', 'user')),
    "adminId" bigint,
    "accountId" bigint,
    action text NOT NULL CHECK (action IN ('create', 'update', 'delete', 'bulk_load')),
    -- NULL for bulk_load, which spans several tables; row counts go to metadata.
    entity text CHECK (entity IN ('account', 'service', 'transaction')),
    "entityId" bigint,
    -- Row state around the operation: NULL before for create, NULL after for delete.
    before jsonb,
    after jsonb,
    outcome text NOT NULL CHECK (outcome IN ('success', 'failure')),
    error text,
    "durationMs" integer CHECK ("durationMs" >= 0),
    -- Groups entries written by one request or one load run.
    "requestId" uuid,
    metadata jsonb NOT NULL DEFAULT '{}',
    CONSTRAINT operation_logs_admin_fk FOREIGN KEY ("adminId")
        REFERENCES public.admins(id) ON UPDATE RESTRICT ON DELETE RESTRICT,
    CONSTRAINT operation_logs_actor_check CHECK (
        ("actorType" = 'admin') = ("adminId" IS NOT NULL)
        AND ("actorType" = 'user') = ("accountId" IS NOT NULL)
    ),
    CONSTRAINT operation_logs_error_check CHECK ((outcome = 'failure') = (error IS NOT NULL)),
    CONSTRAINT operation_logs_target_check CHECK (
        action = 'bulk_load' OR (entity IS NOT NULL AND "entityId" IS NOT NULL)
    )
);

CREATE INDEX operation_logs_occurred_at_idx ON public.operation_logs ("occurredAt");
CREATE INDEX operation_logs_entity_idx ON public.operation_logs (entity, "entityId", "occurredAt");
CREATE INDEX operation_logs_admin_id_idx ON public.operation_logs ("adminId") WHERE "adminId" IS NOT NULL;
CREATE INDEX operation_logs_account_id_idx ON public.operation_logs ("accountId") WHERE "accountId" IS NOT NULL;
CREATE INDEX operation_logs_request_id_idx ON public.operation_logs ("requestId") WHERE "requestId" IS NOT NULL;
CREATE INDEX operation_logs_failures_idx ON public.operation_logs ("occurredAt") WHERE outcome = 'failure';

-- The log is append-only: entries can't be edited or removed.
CREATE FUNCTION public.operation_logs_append_only() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    RAISE EXCEPTION 'operation_logs is append-only (% is not allowed)', TG_OP;
END $$;

CREATE TRIGGER operation_logs_append_only
    BEFORE UPDATE OR DELETE ON public.operation_logs
    FOR EACH ROW EXECUTE FUNCTION public.operation_logs_append_only();

CREATE TRIGGER operation_logs_no_truncate
    BEFORE TRUNCATE ON public.operation_logs
    FOR EACH STATEMENT EXECUTE FUNCTION public.operation_logs_append_only();
