#!/usr/bin/env bash
set -u

project_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$project_dir" || exit 1

if [[ -f .env ]]; then
  set -a
  # shellcheck disable=SC1091
  source .env
  set +a
fi

if [[ -z "${DATABASE_URL:-}" ]]; then
  echo 'DATABASE_URL is missing in .env' >&2
  exit 1
fi

if ! pg_isready -q -d "$DATABASE_URL"; then
  if [[ "$DATABASE_URL" == postgres://user@127.0.0.1:55432/aeza_intelligence && -d .local/postgres ]]; then
    echo 'Starting local PostgreSQL on 127.0.0.1:55432…'
    pg_ctl -D .local/postgres -o '-k /tmp -p 55432 -h 127.0.0.1' -l .local/postgres.log start || exit 1
  else
    echo 'PostgreSQL is unavailable at DATABASE_URL' >&2
    exit 1
  fi
fi

echo 'Checking PostgreSQL connection…'
psql "$DATABASE_URL" -X -q -v ON_ERROR_STOP=1 -c 'SELECT current_database();' || exit 1
cargo run --locked --bin aeza_intelligence -- migrate || exit 1

echo 'Collecting current competitor tariffs…'
cargo run --bin market_import -- run --mode live --max-archives 0
price_status=$?

echo 'Converting imported EUR prices to RUB…'
cargo run --bin market_fx
fx_status=$?

echo 'Rows in PostgreSQL:'
psql "$DATABASE_URL" -X -A -t -v ON_ERROR_STOP=1 -c \
  "SELECT 'tariff_prices=' || count(*) FROM market_intelligence.tariff_prices
   UNION ALL SELECT 'exchange_rates=' || count(*) FROM market_intelligence.exchange_rates
   UNION ALL SELECT 'search_interest=' || count(*) FROM market_intelligence.search_interest;"
count_status=$?

if (( price_status != 0 || fx_status != 0 || count_status != 0 )); then
  echo 'Some collection steps failed. Successful rows, if any, remain in PostgreSQL.' >&2
  exit 1
fi
