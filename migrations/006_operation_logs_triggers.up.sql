-- Logs every insert, update and delete on accounts, services and transactions.
--
-- Who made the change comes from transaction-local settings, set by the application
-- before writing (missing settings mean a system actor):
--   SELECT set_config('app.actor_type', 'admin', true),
--          set_config('app.admin_id', '42', true),
--          set_config('app.request_id', '<uuid>', true);
-- app.actor_type = 'admin' requires app.admin_id, 'user' requires app.account_id;
-- otherwise operation_logs_actor_check rejects the change itself.

CREATE FUNCTION public.log_row_operation() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
    old_row jsonb := CASE WHEN TG_OP <> 'INSERT' THEN to_jsonb(OLD) END;
    new_row jsonb := CASE WHEN TG_OP <> 'DELETE' THEN to_jsonb(NEW) END;
BEGIN
    INSERT INTO public.operation_logs (
        "actorType", "adminId", "accountId", action, entity, "entityId",
        before, after, outcome, "requestId"
    ) VALUES (
        coalesce(nullif(current_setting('app.actor_type', true), ''), 'system'),
        nullif(current_setting('app.admin_id', true), '')::bigint,
        nullif(current_setting('app.account_id', true), '')::bigint,
        CASE TG_OP WHEN 'INSERT' THEN 'create' WHEN 'UPDATE' THEN 'update' ELSE 'delete' END,
        TG_ARGV[0],
        (coalesce(new_row, old_row) ->> 'id')::bigint,
        old_row,
        new_row,
        'success',
        nullif(current_setting('app.request_id', true), '')::uuid
    );
    RETURN NULL;
END $$;

CREATE TRIGGER accounts_log_operation
    AFTER INSERT OR UPDATE OR DELETE ON public.accounts
    FOR EACH ROW EXECUTE FUNCTION public.log_row_operation('account');

CREATE TRIGGER services_log_operation
    AFTER INSERT OR UPDATE OR DELETE ON public.services
    FOR EACH ROW EXECUTE FUNCTION public.log_row_operation('service');

CREATE TRIGGER transactions_log_operation
    AFTER INSERT OR UPDATE OR DELETE ON public.transactions
    FOR EACH ROW EXECUTE FUNCTION public.log_row_operation('transaction');
