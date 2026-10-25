-- DEFAULT_AUTH=true signs admins in with a password instead of OIDC. The hash
-- is an Argon2id PHC string (src/passwords.rs); NULL means no password login.
ALTER TABLE public.admins ADD COLUMN password_hash text;
