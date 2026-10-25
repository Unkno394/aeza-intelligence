-- Local Keycloak demo account; external OIDC users are granted access separately.
INSERT INTO public.admins (email, name, role)
VALUES ('analyst@aeza.local', 'Aéza Analyst', 'superadmin')
ON CONFLICT DO NOTHING;
