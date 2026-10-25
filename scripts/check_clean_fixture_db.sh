#!/usr/bin/env bash
set -euo pipefail

project_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$project_dir"
: "${DATABASE_URL:?Point DATABASE_URL at an empty disposable PostgreSQL database}"
pg_isready -q -d "$DATABASE_URL"

# The script only writes into an empty, disposable database.
if [[ "$(psql "$DATABASE_URL" -X -At -v ON_ERROR_STOP=1 -c \
  "SELECT count(*) FROM public.accounts" 2>/dev/null || echo missing)" != missing ]]; then
  echo 'Expected a new database without public.accounts; refusing to write.' >&2
  exit 1
fi

cargo run --locked --bin aeza_intelligence -- migrate
psql "$DATABASE_URL" -X -q -v ON_ERROR_STOP=1 -f scripts/fixtures/full_model_stress/fixture_insert.sql
psql "$DATABASE_URL" -X -q -v ON_ERROR_STOP=1 -f scripts/seed_price_impact_demo.sql
psql "$DATABASE_URL" -X -q -v ON_ERROR_STOP=1 -f scripts/fixtures/edge_cases.sql

tmp_dir="$(mktemp -d)"
trap 'rm -rf "$tmp_dir"' EXIT
scripts/generate_full_model_fixture.sh "$tmp_dir" >/dev/null
for name in data.json fixture_insert.sql model_history.json; do
  cmp "scripts/fixtures/full_model_stress/$name" "$tmp_dir/$name"
done
cargo run --locked --bin model_smoke -- scripts/fixtures/full_model_stress/model_history.json

counts="$(psql "$DATABASE_URL" -X -At -v ON_ERROR_STOP=1 -c \
  "SELECT (SELECT count(*) FROM public.accounts), (SELECT count(*) FROM public.services),
          (SELECT count(*) FROM public.transactions)")"
[[ "$counts" == '1506|2113|7788' ]] || {
  echo "Unexpected fixture row counts: $counts" >&2; exit 1;
}

features="$(psql "$DATABASE_URL" -X -At -v ON_ERROR_STOP=1 -c \
  "SELECT \"accountId\" || ':' || \"servicesCount\" || ':' || \"upcomingPayments\" || ':' ||
          coalesce(\"paymentCoverage\"::text, 'null')
   FROM public.client_features(now(), '7 days')
   WHERE \"accountId\" BETWEEN -9800004 AND -9800001 ORDER BY \"accountId\"")"
[[ "$features" == $'-9800004:1:0:null\n-9800003:1:500:0\n-9800002:0:0:null\n-9800001:0:0:null' ]] || {
  echo "Unexpected edge-case features: $features" >&2; exit 1;
}

payment_terms="$(psql "$DATABASE_URL" -X -At -v ON_ERROR_STOP=1 -c \
  "SELECT string_agg(public.payment_term_seconds(term)::bigint::text, '|' ORDER BY position)
   FROM (VALUES (1, 'hour'), (2, 'week'), (3, 'month'), (4, 'quarter_year'),
                (5, 'half_year'), (6, 'year')) AS terms(position, term)")"
[[ "$payment_terms" == '3600|604800|2592000|7862400|15724800|31536000' ]] || {
  echo "Unexpected payment-term seconds: $payment_terms" >&2; exit 1;
}

billing="$(psql "$DATABASE_URL" -X -At -v ON_ERROR_STOP=1 -c \
  "SELECT \"servicesCount\" || '|' || \"upcomingPayments\" || '|' || \"spendableBalance\" || '|' ||
          round(\"monthlySpend\"::numeric, 2)
   FROM public.client_features('2000-01-01T00:00:00Z', '7 days') WHERE \"accountId\" = -9800005")"
[[ "$billing" == '8|15400|150|73086.24' ]] || {
  echo "Unexpected payment projection/status/monthly revenue result: $billing" >&2; exit 1;
}

bonus_balances="$(psql "$DATABASE_URL" -X -At -v ON_ERROR_STOP=1 -c \
  "SELECT string_agg(\"spendableBalance\"::text, '|' ORDER BY \"accountId\" DESC)
   FROM public.client_features(now(), '7 days') WHERE \"accountId\" IN (-9800005, -9800006)")"
[[ "$bonus_balances" == '150|50' ]] || {
  echo "Unlocked/locked bonus or referral balance handling is incorrect: $bonus_balances" >&2; exit 1;
}

python3 - "$tmp_dir/price_check.sql" <<'PY'
from pathlib import Path
import sys
query = Path('src/sql/internal_price_impact.sql').read_text().strip().rstrip(';')
Path(sys.argv[1]).write_text('SELECT count(*), count(control_account_id), '
    'count(*) FILTER (WHERE complete_pretrend AND complete_followup AND renewal_due_in_window) '
    'FROM (' + query + ') events;\n')
PY
price_counts="$(psql "$DATABASE_URL" -X -At -v ON_ERROR_STOP=1 -f "$tmp_dir/price_check.sql")"
[[ "$price_counts" == '600|600|600' ]] || {
  echo "Unexpected price cohort counts: $price_counts" >&2; exit 1;
}

# Exercise the daily collector over the full generated history dataset. A second
# run on the same UTC day must update the same account/day rows, not duplicate them.
DATABASE_URL="$DATABASE_URL" cargo run --locked --bin client_snapshot
snapshots_after_collect="$(psql "$DATABASE_URL" -X -At -v ON_ERROR_STOP=1 -c \
  'SELECT count(*) FROM public.client_snapshots')"
DATABASE_URL="$DATABASE_URL" cargo run --locked --bin client_snapshot
snapshots_after_replay="$(psql "$DATABASE_URL" -X -At -v ON_ERROR_STOP=1 -c \
  'SELECT count(*) FROM public.client_snapshots')"
[[ "$snapshots_after_collect" == "$snapshots_after_replay" ]] || {
  echo "Daily collector duplicated snapshots on replay: $snapshots_after_collect → $snapshots_after_replay" >&2; exit 1;
}
duplicate_days="$(psql "$DATABASE_URL" -X -At -v ON_ERROR_STOP=1 -c \
  'SELECT count(*) FROM (SELECT account_id, observed_on FROM public.client_snapshots GROUP BY 1, 2 HAVING count(*) > 1) duplicates')"
[[ "$duplicate_days" == 0 ]] || {
  echo "Found duplicate account/day snapshots: $duplicate_days" >&2; exit 1;
}
echo "Fixture reproduction passed: accounts/services/transactions=$counts; price=$price_counts; payment terms, statuses and spendable balances verified"
