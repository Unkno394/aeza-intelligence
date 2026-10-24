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
: "${DATABASE_URL:?Set DATABASE_URL to the existing PostgreSQL database}"
pg_isready -q -d "$DATABASE_URL" || { echo 'PostgreSQL is unavailable' >&2; exit 1; }

cargo run --locked --bin aeza_intelligence -- migrate
cargo run --locked --bin market_import -- aeza-snapshot
cargo run --locked --bin market_import -- run --provider contabo --mode live --max-archives 0
cargo run --locked --bin market_fx

psql "$DATABASE_URL" -X -v ON_ERROR_STOP=1 -c "
SELECT provider.code, count(DISTINCT product.id) AS products,
       count(DISTINCT price.id) AS price_observations,
       count(DISTINCT price.id) FILTER (WHERE price.currency IN ('EUR','USD')
            AND price.price_rub_minor IS NULL) AS missing_fx
FROM market_intelligence.providers provider
JOIN market_intelligence.tariff_products product ON product.provider_id = provider.id
JOIN market_intelligence.provider_tariffs tariff ON tariff.product_id = product.id
JOIN market_intelligence.tariff_prices_rub price ON price.provider_tariff_id = tariff.id
WHERE provider.code IN ('aeza','contabo')
GROUP BY provider.code ORDER BY provider.code;"
fx_errors="$(psql "$DATABASE_URL" -X -At -v ON_ERROR_STOP=1 -c "
SELECT count(*) FROM market_intelligence.tariff_prices_rub
WHERE currency IN ('EUR','USD') AND rub_rate IS NOT NULL
  AND price_rub_minor IS DISTINCT FROM round(price_minor * rub_rate)::bigint;")"
if [[ "$fx_errors" != 0 ]]; then
  echo "Currency conversion mismatches: $fx_errors" >&2
  exit 1
fi
psql "$DATABASE_URL" -X -v ON_ERROR_STOP=1 -c "
SELECT source, comparison_group, count(DISTINCT brand_code) AS brands
FROM market_intelligence.search_interest
GROUP BY source, comparison_group ORDER BY source, comparison_group;"

api_port="${MARKET_CHECK_PORT:-3011}"
check_dir="$(mktemp -d /tmp/aeza-market-check.XXXXXX)"
cargo run --quiet --locked --bin market_api -- --listen "127.0.0.1:${api_port}" &
api_pid=$!
trap 'kill "$api_pid" 2>/dev/null || true' EXIT
api_ready=0
for _ in {1..30}; do
  if curl -fs "http://127.0.0.1:${api_port}/api/market/prices/quality" -o "$check_dir/quality.json"; then
    api_ready=1
    break
  fi
  sleep 1
done
if [[ "$api_ready" != 1 ]]; then
  echo 'New Market API did not become ready on the check port' >&2
  exit 1
fi

curl -fsS "http://127.0.0.1:${api_port}/api/market/prices/monitor" -o "$check_dir/monitor.json"
curl -fsS "http://127.0.0.1:${api_port}/api/market/prices/history" -o "$check_dir/history.json"
curl -fsS "http://127.0.0.1:${api_port}/api/client/data-quality" -o "$check_dir/clients.json"
AEZA_CHECK_DIR="$check_dir" python3 - <<'PY'
import json
import os
from pathlib import Path

directory = Path(os.environ['AEZA_CHECK_DIR'])
quality = json.loads((directory / 'quality.json').read_text())
monitor = json.loads((directory / 'monitor.json').read_text())
history = json.loads((directory / 'history.json').read_text())
clients = json.loads((directory / 'clients.json').read_text())
contabo = next((p for p in quality if p['providerCode'] == 'contabo'), None)
assert contabo and contabo['completePrices'] > 0, 'Contabo has no complete prices'
assert contabo['missingFxPrices'] == 0, 'Contabo has prices without RUB conversion'
assert any(p['providerCode'] == 'contabo' and p['priceRub'] is not None for p in monitor), 'Contabo is absent from monitoring API'
assert any(p['providerCode'] == 'contabo' for p in history), 'Contabo is absent from history API'
print(f"Contabo: {contabo['completePrices']} complete prices; FX missing: {contabo['missingFxPrices']}")
print(f"Price APIs: {len(monitor)} current tariffs, {len(history)} history points")
print(f"Real clients: {clients['realAccounts']}; enough history: {clients['realAccountsWithHistory']}; unlinked services: {clients['unlinkedServices']}")
PY
