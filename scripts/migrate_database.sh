#!/usr/bin/env bash
set -euo pipefail

project_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$project_dir"

if [[ -f .env ]]; then
  set -a
  # shellcheck disable=SC1091
  source .env
  set +a
fi
: "${DATABASE_URL:?Set DATABASE_URL to the existing PostgreSQL database in .env}"

if ! pg_isready -q -d "$DATABASE_URL"; then
  echo 'Existing PostgreSQL database is unavailable at DATABASE_URL.' >&2
  exit 1
fi

cargo run --bin aeza_intelligence -- migrate

psql "$DATABASE_URL" -X -v ON_ERROR_STOP=1 <<'SQL'
DO $$
DECLARE
    missing text;
BEGIN
    SELECT string_agg(name, ', ' ORDER BY name) INTO missing
    FROM unnest(ARRAY[
        'public.accounts', 'public.services', 'public.transactions',
        'public.admins', 'public.operation_logs', 'public.client_snapshots',
        'public.client_settings',
        'public.financial_ledger',
        'market_intelligence.providers',
        'market_intelligence.provider_tariffs',
        'market_intelligence.tariff_products',
        'market_intelligence.tariff_prices',
        'market_intelligence.tariff_data_quality',
        'market_intelligence.search_brands',
        'market_intelligence.search_interest',
        'market_intelligence.exchange_rates',
        'market_intelligence.search_interest_geo'
    ]) AS required(name)
    WHERE to_regclass(name) IS NULL;
    IF missing IS NOT NULL THEN
        RAISE EXCEPTION 'Missing tables after migration: %', missing;
    END IF;
    IF NOT EXISTS (SELECT 1 FROM information_schema.columns
                   WHERE table_schema = 'public' AND table_name = 'services'
                     AND column_name = 'tariff_product_id')
       OR NOT EXISTS (SELECT 1 FROM information_schema.columns
                   WHERE table_schema = 'market_intelligence'
                     AND table_name = 'search_interest'
                     AND column_name = 'comparison_group') THEN
        RAISE EXCEPTION 'Missing tariff identity or search scale column after migration';
    END IF;
END $$;

SELECT current_database() AS database,
       (SELECT count(*) FROM public.accounts) AS accounts,
       (SELECT count(*) FROM public.client_snapshots) AS client_snapshots,
       (SELECT count(*) FROM market_intelligence.tariff_prices) AS tariff_prices,
       (SELECT count(*) FROM market_intelligence.search_interest) AS search_interest;
SQL
