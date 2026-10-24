-- Preserve an existing client schema created by the former scripts/001_schema.sql.
-- A partial schema needs manual inspection rather than silently creating a mix
-- of old and new tables.
DO $$
DECLARE
    present_count integer;
    missing_columns text;
    old_constraint text;
BEGIN
    SELECT (to_regclass('public.accounts') IS NOT NULL)::integer
         + (to_regclass('public.services') IS NOT NULL)::integer
         + (to_regclass('public.transactions') IS NOT NULL)::integer
    INTO present_count;

    IF present_count NOT IN (0, 3) THEN
        RAISE EXCEPTION 'Partial legacy client schema: inspect accounts, services and transactions before migrating';
    END IF;
    IF present_count = 0 THEN
        RETURN;
    END IF;

    SELECT string_agg(required.table_name || '.' || required.column_name, ', ')
    INTO missing_columns
    FROM (VALUES
        ('accounts', 'id'), ('accounts', 'email'), ('accounts', 'balance'),
        ('accounts', 'withdrawBalance'), ('accounts', 'totalReplenished'),
        ('accounts', 'bonusBalance'), ('accounts', 'bonusState'),
        ('services', 'id'), ('services', 'name'), ('services', 'ownerId'),
        ('services', 'paymentTerm'), ('services', 'autoProlong'),
        ('services', 'createdAt'), ('services', 'expiresAt'),
        ('services', 'status'), ('services', 'price'),
        ('transactions', 'id'), ('transactions', 'amount'),
        ('transactions', 'status'), ('transactions', 'performedAt'),
        ('transactions', 'createdAt'), ('transactions', 'type'),
        ('transactions', 'serviceId')
    ) AS required(table_name, column_name)
    WHERE NOT EXISTS (
        SELECT 1 FROM information_schema.columns c
        WHERE c.table_schema = 'public'
          AND c.table_name = required.table_name
          AND c.column_name = required.column_name
    );
    IF missing_columns IS NOT NULL THEN
        RAISE EXCEPTION 'Existing client schema is missing columns: %', missing_columns;
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM information_schema.columns
        WHERE table_schema = 'public' AND table_name = 'accounts'
          AND column_name = 'id' AND data_type IN ('integer', 'bigint')
    ) OR NOT EXISTS (
        SELECT 1 FROM information_schema.columns
        WHERE table_schema = 'public' AND table_name = 'services'
          AND column_name = 'ownerId' AND data_type IN ('integer', 'bigint')
    ) OR NOT EXISTS (
        SELECT 1 FROM information_schema.columns
        WHERE table_schema = 'public' AND table_name = 'transactions'
          AND column_name = 'serviceId' AND data_type IN ('integer', 'bigint')
    ) THEN
        RAISE EXCEPTION 'Unsupported client key columns: inspect the existing schema before migrating';
    END IF;

    IF EXISTS (
        SELECT 1 FROM information_schema.columns
        WHERE table_schema = 'public' AND table_name = 'accounts'
          AND column_name = 'bonusState' AND data_type = 'USER-DEFINED'
    ) THEN
        ALTER TABLE public.accounts ALTER COLUMN "bonusState" DROP DEFAULT;
        ALTER TABLE public.accounts ALTER COLUMN "bonusState" TYPE text USING "bonusState"::text;
        ALTER TABLE public.accounts ALTER COLUMN "bonusState" SET DEFAULT 'locked';
    END IF;
    IF EXISTS (
        SELECT 1 FROM information_schema.columns
        WHERE table_schema = 'public' AND table_name = 'services'
          AND column_name = 'paymentTerm' AND data_type = 'USER-DEFINED'
    ) THEN
        ALTER TABLE public.services ALTER COLUMN "paymentTerm" TYPE text USING "paymentTerm"::text;
    END IF;
    IF EXISTS (
        SELECT 1 FROM information_schema.columns
        WHERE table_schema = 'public' AND table_name = 'services'
          AND column_name = 'status' AND data_type = 'USER-DEFINED'
    ) THEN
        ALTER TABLE public.services ALTER COLUMN status TYPE text USING status::text;
    END IF;
    IF EXISTS (
        SELECT 1 FROM information_schema.columns
        WHERE table_schema = 'public' AND table_name = 'transactions'
          AND column_name = 'status' AND data_type = 'USER-DEFINED'
    ) THEN
        -- The former schema has an unnamed CHECK comparing its enum status.
        -- PostgreSQL cannot retype that expression automatically.
        FOR old_constraint IN
            SELECT conname FROM pg_constraint
            WHERE conrelid = 'public.transactions'::regclass
              AND contype = 'c'
              AND pg_get_constraintdef(oid) LIKE '%status%'
        LOOP
            EXECUTE format('ALTER TABLE public.transactions DROP CONSTRAINT %I', old_constraint);
        END LOOP;
        ALTER TABLE public.transactions ALTER COLUMN status TYPE text USING status::text;
        ALTER TABLE public.transactions
            ADD CONSTRAINT legacy_transactions_performed_check
            CHECK ((status = 'performed') = ("performedAt" IS NOT NULL));
    END IF;
    IF EXISTS (
        SELECT 1 FROM information_schema.columns
        WHERE table_schema = 'public' AND table_name = 'transactions'
          AND column_name = 'type' AND data_type = 'USER-DEFINED'
    ) THEN
        ALTER TABLE public.transactions ALTER COLUMN type TYPE text USING type::text;
    END IF;
END $$;
