#!/usr/bin/env bash
set -euo pipefail

project_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$project_dir"
: "${DATABASE_URL:?Set DATABASE_URL to the disposable Compose test database}"
: "${PUBLIC_URL:?Set PUBLIC_URL to the disposable Compose application URL}"
: "${E2E_USERNAME:?Set E2E_USERNAME to an active test operator}"

session_token="$(openssl rand -hex 32)"
session_minutes="${LOAD_SESSION_MINUTES:-10}"
[[ "$session_minutes" =~ ^[1-9][0-9]*$ ]] || { echo 'LOAD_SESSION_MINUTES must be a positive integer' >&2; exit 1; }
session_id="$(psql "$DATABASE_URL" -X -At -q -v ON_ERROR_STOP=1 \
  -v session_token="$session_token" -v email="$E2E_USERNAME" -v session_minutes="$session_minutes" -c \
  "INSERT INTO public.admin_sessions (token_hash, admin_id, expires_at) \
   SELECT sha256(convert_to(:'session_token', 'UTF8')), id, now() + make_interval(mins => :'session_minutes'::int) \
   FROM public.admins WHERE lower(email) = lower(:'email') AND \"isActive\" \
   RETURNING token_hash")"
[[ -n "$session_id" ]] || { echo 'Could not issue a temporary performance-test session' >&2; exit 1; }
export AEZA_LOAD_COOKIE="aeza_session=$session_token"

tmp_dir="$(mktemp -d)"
load_pid=""
cleanup() {
  if [[ -n "$load_pid" ]] && kill -0 "$load_pid" 2>/dev/null; then
    kill "$load_pid" 2>/dev/null || true
    wait "$load_pid" 2>/dev/null || true
  fi
  psql "$DATABASE_URL" -X -q -v ON_ERROR_STOP=1 -v session_token="$session_token" -c \
    "DELETE FROM public.admin_sessions WHERE token_hash = sha256(convert_to(:'session_token', 'UTF8'))" \
    >/dev/null 2>&1 || true
  rm -rf "$tmp_dir"
}
trap cleanup EXIT

api_container="$(docker compose ps -q api)"
[[ -n "$api_container" ]] || { echo 'Compose API container is not running' >&2; exit 1; }

node scripts/performance_load.mjs >"$tmp_dir/report.json" &
load_pid=$!
while kill -0 "$load_pid" 2>/dev/null; do
  docker stats "$api_container" --no-stream --format '{{.CPUPerc}},{{.MemUsage}}' >>"$tmp_dir/api-resources.csv"
  sleep 5
done
if wait "$load_pid"; then
  cat "$tmp_dir/report.json"
else
  cat "$tmp_dir/report.json" >&2
  echo 'Concurrent API performance checks failed' >&2
  exit 1
fi

resource_samples="$(wc -l <"$tmp_dir/api-resources.csv" | tr -d ' ')"
(( resource_samples > 0 )) || { echo 'No API CPU/memory samples were recorded' >&2; exit 1; }
echo "API CPU/memory samples: $resource_samples"
cat "$tmp_dir/api-resources.csv"

# Long runs provide enough samples to detect sustained API memory growth after
# warmup. Short CI runs still report resource samples without claiming that
# they establish a leak trend.
python3 - "$tmp_dir/api-resources.csv" "${API_MEMORY_GROWTH_LIMIT_MB:-128}" <<'PY'
import csv
import re
import statistics
import sys

path, limit_mb = sys.argv[1], float(sys.argv[2])
units = {'B': 1, 'kB': 1000, 'KB': 1000, 'KiB': 1024, 'MB': 1000**2,
         'MiB': 1024**2, 'GB': 1000**3, 'GiB': 1024**3}
samples = []
with open(path, newline='') as source:
    for row in csv.reader(source):
        if len(row) < 2:
            continue
        match = re.match(r'\s*([0-9.]+)\s*([A-Za-z]+)', row[1].split('/', 1)[0])
        if match and match.group(2) in units:
            samples.append(float(match.group(1)) * units[match.group(2)])

if len(samples) < 20:
    print(f'Memory trend check skipped: {len(samples)} samples; extended soak needs at least 20.')
else:
    window = max(3, len(samples) // 5)
    warm = statistics.median(samples[:window])
    settled = statistics.median(samples[-window:])
    growth_mb = (settled - warm) / (1024 * 1024)
    print(f'API memory trend after warmup: {growth_mb:.1f} MiB (limit {limit_mb:.1f} MiB)')
    if growth_mb > limit_mb:
        raise SystemExit('API memory grew beyond the configured soak limit')
PY
