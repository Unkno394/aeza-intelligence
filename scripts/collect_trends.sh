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
: "${DATABASE_URL:?DATABASE_URL is required in .env}"

for argument in "$@"; do
  if [[ "$argument" == *.csv || "$argument" == *.CSV ]]; then
    if [[ ! -f "$argument" ]]; then
      echo "CSV file not found: $argument" >&2
      echo 'Download the Interest over time and Interest by region CSV files from Google Trends first.' >&2
      exit 1
    fi
  fi
done

if ! pg_isready -q -d "$DATABASE_URL"; then
  if [[ "$DATABASE_URL" == postgres://user@127.0.0.1:55432/aeza_intelligence && -d .local/postgres ]]; then
    pg_ctl -D .local/postgres -o '-k /tmp -p 55432 -h 127.0.0.1' -l .local/postgres.log start
  else
    echo 'PostgreSQL is unavailable at DATABASE_URL' >&2
    exit 1
  fi
fi

bash scripts/migrate_database.sh

if [[ $# -eq 0 ]]; then
  cargo run --bin search_interest -- once
  psql "$DATABASE_URL" -X -A -t -v ON_ERROR_STOP=1 -c \
    "SELECT brand_code || '=' || count(*) FROM market_intelligence.search_interest WHERE source = 'google search' GROUP BY brand_code ORDER BY brand_code;"
else
  cargo run --bin google_trends_import -- "$@"
  psql "$DATABASE_URL" -X -A -t -v ON_ERROR_STOP=1 -c \
    "SELECT 'weekly_points=' || count(*) FROM market_intelligence.search_interest WHERE source = 'google trends compare 5y'
     UNION ALL SELECT 'country_points=' || count(*) FROM market_intelligence.search_interest_geo WHERE source = 'google trends compare 5y';"
fi
