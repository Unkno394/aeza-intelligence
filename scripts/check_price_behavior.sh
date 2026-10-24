#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
if [[ -f .env ]]; then set -a; source .env; set +a; fi
: "${DATABASE_URL:?DATABASE_URL is required}"
pg_isready -q -d "$DATABASE_URL" || { echo 'PostgreSQL недоступна' >&2; exit 1; }

cargo run --quiet --locked --bin aeza_intelligence -- migrate
psql "$DATABASE_URL" -X -v ON_ERROR_STOP=1 -c "
SELECT count(*) FILTER (WHERE s.tariff_product_id IS NOT NULL) AS linked,
       count(*) FILTER (WHERE s.tariff_product_id IS NULL) AS unlinked,
       count(*) FILTER (WHERE s.tariff_product_id IS NOT NULL AND p.code <> 'aeza') AS wrong_provider
FROM public.services s
LEFT JOIN market_intelligence.tariff_products t ON t.id = s.tariff_product_id
LEFT JOIN market_intelligence.providers p ON p.id = t.provider_id
WHERE s.\"ownerId\" >= 0;"
wrong_provider="$(psql "$DATABASE_URL" -X -At -v ON_ERROR_STOP=1 -c "
SELECT count(*) FROM public.services s
JOIN market_intelligence.tariff_products t ON t.id = s.tariff_product_id
JOIN market_intelligence.providers p ON p.id = t.provider_id
WHERE s.\"ownerId\" >= 0 AND p.code <> 'aeza';")"
if [[ "$wrong_provider" != 0 ]]; then
  echo "Ошибочно связанные с конкурентами услуги: $wrong_provider" >&2
  exit 1
fi
psql "$DATABASE_URL" -X -v ON_ERROR_STOP=1 -c "
SELECT t.plan_key, count(*) AS services, count(DISTINCT s.\"ownerId\") AS accounts,
       count(*) FILTER (WHERE s.status = 'prolong_wait') AS prolong_wait,
       count(*) FILTER (WHERE s.status IN ('deleted','blocked','suspended')) AS inactive
FROM public.services s JOIN market_intelligence.tariff_products t ON t.id = s.tariff_product_id
WHERE s.\"ownerId\" >= 0 GROUP BY t.plan_key ORDER BY services DESC LIMIT 20;"
expected="$(psql "$DATABASE_URL" -X -At -v ON_ERROR_STOP=1 -c "
SELECT count(*),
       count(*) FILTER (WHERE status = 'prolong_wait'),
       count(*) FILTER (WHERE status IN ('deleted','blocked','suspended')),
       (SELECT count(*) FROM public.transactions tx
        JOIN public.services sx ON sx.id = tx.\"serviceId\"
        JOIN market_intelligence.tariff_products px ON px.id = sx.tariff_product_id
        JOIN market_intelligence.providers vx ON vx.id = px.provider_id
        WHERE sx.\"ownerId\" >= 0 AND vx.code = 'aeza'
          AND tx.type = 'prolong' AND tx.status = 'performed')
FROM public.services s
JOIN market_intelligence.tariff_products t ON t.id = s.tariff_product_id
JOIN market_intelligence.providers p ON p.id = t.provider_id
WHERE s.\"ownerId\" >= 0 AND p.code = 'aeza';")"

port="${MARKET_CHECK_PORT:-3012}"
tmp="$(mktemp -d)"
cargo run --quiet --locked --bin market_api -- --listen "127.0.0.1:$port" >"$tmp/api.log" 2>&1 &
pid=$!
trap 'kill "$pid" 2>/dev/null || true; rm -rf "$tmp"' EXIT
ready=0
for _ in {1..30}; do
  if curl -fsS "http://127.0.0.1:$port/api/market/prices/client-cohorts" -o "$tmp/cohorts.json"; then ready=1; break; fi
  sleep 1
done
if [[ "$ready" != 1 ]]; then cat "$tmp/api.log" >&2; exit 1; fi
curl -fsS "http://127.0.0.1:$port/api/market/prices/behavior" -o "$tmp/behavior.json"
CHECK_DIR="$tmp" EXPECTED_COUNTS="$expected" python3 - <<'PY'
import json, os
from pathlib import Path
p = Path(os.environ['CHECK_DIR'])
cohorts = json.loads((p/'cohorts.json').read_text())
rows = json.loads((p/'behavior.json').read_text())
assert isinstance(cohorts, list) and isinstance(rows, list)
for c in cohorts:
    assert 0 <= c['awaitingRenewal'] <= c['linkedServices']
    assert 0 <= c['inactiveServices'] <= c['linkedServices']
    assert 0 <= c['accounts'] <= c['linkedServices']
expected = tuple(map(int, os.environ['EXPECTED_COUNTS'].split('|')))
actual = (sum(c['linkedServices'] for c in cohorts),
          sum(c['awaitingRenewal'] for c in cohorts),
          sum(c['inactiveServices'] for c in cohorts),
          sum(c['performedRenewals'] for c in cohorts))
assert actual == expected, f'API/DB service count mismatch: {actual} != {expected}'
for r in rows:
    assert r['period'] in ('before', 'after') and r['cohort'] in ('tariff', 'peer')
    assert r['services'] >= r['accounts'] >= 0
    assert r['performedRenewals'] >= 0 and r['prolongWaitEntries'] >= 0
    assert 0 <= r['prolongWaitServices'] <= r['services']
    assert r['previousMinor'] != r['currentMinor']
print(f"API: {len(cohorts)} тарифных групп, {len(rows)} строк анализа цены и поведения")
if not rows:
    print('Изменений цены с двумя наблюдениями пока нет; исторический вывод недоступен')
elif not any(r['completeFollowup'] and r['completeEventCoverage'] and r['services'] >= 5 and r['performedRenewals'] >= 3 for r in rows):
    print('Есть изменения цены, но пока недостаточно полного периода/журнала/связанных услуг')
PY
