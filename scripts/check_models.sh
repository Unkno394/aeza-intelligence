#!/usr/bin/env bash
set -euo pipefail

project_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$project_dir"

fixture_dir="$(mktemp -d /tmp/aeza-model-check.XXXXXX)"
python3 scripts/generate.py --accounts 10 --services 30 --transactions 100 \
  --history-days 60 --id-offset -9000000 --out "$fixture_dir"

if [[ "${1:-}" != "--persist" ]]; then
  cargo run --offline --locked --bin model_smoke -- "$fixture_dir/model_history.json"
  echo 'Offline model check passed. Use --persist only with a fixture database.'
  exit 0
fi

if [[ -f .env ]]; then
  set -a
  # shellcheck disable=SC1091
  source .env
  set +a
fi
: "${DATABASE_URL:?Set DATABASE_URL before --persist}"
if ! pg_isready -q -d "$DATABASE_URL"; then
  echo 'PostgreSQL is unavailable; no fixture data was written.' >&2
  exit 1
fi

# Negative fixture IDs leave the normal identity sequences and existing clients alone.
cargo run --locked --bin aeza_intelligence -- migrate
psql "$DATABASE_URL" -X -v ON_ERROR_STOP=1 -q -f "$fixture_dir/fixture_insert.sql"
cargo run --locked --bin model_smoke -- "$fixture_dir/model_history.json" --persist

snapshot_count="$(psql "$DATABASE_URL" -X -At -v ON_ERROR_STOP=1 -c \
  'SELECT count(*) FROM public.client_snapshots WHERE account_id = -8999999 AND assessment IS NOT NULL AND model_state IS NOT NULL')"
if (( snapshot_count < 60 )); then
  echo "PostgreSQL contains only $snapshot_count historical snapshots for the fixture account" >&2
  exit 1
fi
echo "PostgreSQL: $snapshot_count persisted historical snapshots for account -8999999"

# A scheduler restart on the same UTC day must not create a second row.
cargo run --quiet --locked --bin client_snapshot
cargo run --quiet --locked --bin client_snapshot
today_count="$(psql "$DATABASE_URL" -X -At -v ON_ERROR_STOP=1 -c \
  "SELECT count(*) FROM public.client_snapshots WHERE account_id = -8999999 AND observed_on = (now() AT TIME ZONE 'UTC')::date")"
if [[ "$today_count" != "1" ]]; then
  echo "Daily scheduler check failed: $today_count rows for fixture account today" >&2
  exit 1
fi
echo 'Scheduler: one UTC-day snapshot after two runs'

# Four separate process starts: first day, duplicate, next day, duplicate.
for instant in 2026-10-20T12:00:00Z 2026-10-20T12:00:00Z \
               2026-10-21T12:00:00Z 2026-10-21T12:00:00Z; do
  cargo run --quiet --locked --bin client_snapshot -- \
    --as-of "$instant" --account-id=-8999999
done
clock_check="$(psql "$DATABASE_URL" -X -At -v ON_ERROR_STOP=1 -c "
  SELECT (SELECT count(*) FROM public.client_snapshots WHERE account_id = -8999999 AND observed_on = DATE '2026-10-20') || ',' ||
         (SELECT count(*) FROM public.client_snapshots WHERE account_id = -8999999 AND observed_on = DATE '2026-10-21') || ',' ||
         ((SELECT (model_state->>'observation_count')::int FROM public.client_snapshots WHERE account_id = -8999999 AND observed_on = DATE '2026-10-21') -
          (SELECT (model_state->>'observation_count')::int FROM public.client_snapshots WHERE account_id = -8999999 AND observed_on = DATE '2026-10-20'))")"
if [[ "$clock_check" != "1,1,1" ]]; then
  echo "Controlled-clock or persisted-state check failed: $clock_check" >&2
  exit 1
fi
echo 'Scheduler: one snapshot per day across midnight and process restarts; model state advanced once'

duplicate_groups="$(psql "$DATABASE_URL" -X -At -v ON_ERROR_STOP=1 -c \
  'SELECT count(*) FROM (SELECT account_id, observed_on FROM public.client_snapshots GROUP BY account_id, observed_on HAVING count(*) > 1) AS duplicates')"
if [[ "$duplicate_groups" != "0" ]]; then
  echo "Duplicate snapshot groups: $duplicate_groups" >&2
  exit 1
fi

api_base="${MARKET_API_BASE_URL:-http://127.0.0.1:3000}"
curl -fsS "$api_base/api/client/analysis" | python3 -c \
  'import json,sys; rows=json.load(sys.stdin)["clients"]; assert len(rows)>=10; print(f"Snapshot API: {len(rows)} clients")'
curl -fsS -X POST "$api_base/api/client/snapshot" -H 'Content-Type: application/json' \
  --data '{"weights":{"paymentCoverage":35,"noAutoProlong":15,"prolongWait":15,"failedProlong":10,"expiringRevenue":25},"mediumThreshold":35,"highThreshold":65,"renewalWindowDays":14}' \
  | python3 -c 'import json,sys; rows=json.load(sys.stdin); assert len(rows)>=10 and all(0<=r["score"]<=100 for r in rows); print(f"Risk Score API: {len(rows)} clients")'
curl -fsS "$api_base/api/client/historical/-8999999" | python3 -c \
  'import json,sys; rows=json.load(sys.stdin); assert len(rows)>=60; print(f"Historical API: {len(rows)} days for fixture account")'
curl -fsS "$api_base/api/client/historical/patterns" | python3 -c \
  'import json,sys; rows=json.load(sys.stdin); assert isinstance(rows,list); assert all(row["support"]>=2 and row["support"]==len(row["clientIds"]) for row in rows); print(f"Sequence discovery API: {len(rows)} repeated patterns")'

python3 scripts/check_frontend_models.py "${FRONTEND_BASE_URL:-http://127.0.0.1:${FRONTEND_PORT:-8080}}"
