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

: "${DATABASE_URL:?Set DATABASE_URL to an isolated integration database}"
frontend_port="${FRONTEND_PORT:-8080}"
keycloak_port="${KEYCLOAK_PORT:-5556}"

docker compose config -q
pg_isready -q -d "$DATABASE_URL"
psql "$DATABASE_URL" -X -At -v ON_ERROR_STOP=1 -c \
  'SELECT count(*) FROM public._sqlx_migrations WHERE success' | grep -Eq '^[1-9][0-9]*$'

wait_for_http() {
  local url="$1" expected="$2" label="$3"
  for _ in $(seq 1 60); do
    local status
    status="$(curl -sS -o /dev/null -w '%{http_code}' --max-time 3 "$url" || true)"
    [[ "$status" == "$expected" ]] && return 0
    sleep 1
  done
  echo "$label did not become ready (last HTTP $status): $url" >&2
  return 1
}

app_url="http://127.0.0.1:${frontend_port}"
keycloak_url="http://127.0.0.1:${keycloak_port}"
wait_for_http "$app_url/" 200 'frontend'
wait_for_http "$app_url/api/v1/openapi.json" 200 'API'
wait_for_http "$keycloak_url/realms/aeza/.well-known/openid-configuration" 200 'Keycloak realm'

required_services=(keycloak api frontend simulator snapshots market-collector)
running_services="$(docker compose ps --services --filter status=running)"
for service in "${required_services[@]}"; do
  grep -Fxq "$service" <<<"$running_services" || {
    echo "required service is not running: $service" >&2
    exit 1
  }
done

settings_fingerprint() {
  psql "$DATABASE_URL" -X -At -v ON_ERROR_STOP=1 -c \
    "SELECT version || ':' || md5(settings::text) FROM public.client_settings WHERE id"
}
snapshot_marker() {
  psql "$DATABASE_URL" -X -At -v ON_ERROR_STOP=1 -c \
    'SELECT coalesce(max((extract(epoch FROM observed_at) * 1000)::bigint), 0) FROM public.client_snapshots'
}
before_restart="$(settings_fingerprint)"
[[ -n "$before_restart" ]] || { echo 'model settings were not persisted before restart' >&2; exit 1; }

# A bad auth configuration must fail closed during startup.
failure_log="$(mktemp)"
if docker compose run --rm --no-deps \
  -e OIDC_ISSUER= -e OIDC_CLIENT_ID= \
  -e LOCAL_ADMIN_USER= -e LOCAL_ADMIN_PASSWORD= \
  api --listen 127.0.0.1:13001 >"$failure_log" 2>&1; then
  rm -f "$failure_log"
  echo 'API unexpectedly started without an OIDC issuer or local admin credentials' >&2
  exit 1
fi
grep -q 'OIDC_ISSUER or LOCAL_ADMIN_USER/LOCAL_ADMIN_PASSWORD is required' "$failure_log" || {
  cat "$failure_log" >&2
  rm -f "$failure_log"
  echo 'API failed for a reason other than the intentionally invalid auth configuration' >&2
  exit 1
}
rm -f "$failure_log"

# Stop and recreate the application stack while the PostgreSQL service is
# restarted separately, then prove persisted configuration and dependencies recover.
docker compose stop api frontend keycloak simulator snapshots market-collector
stale_snapshot_account="$(psql "$DATABASE_URL" -X -At -q -v ON_ERROR_STOP=1 -c \
  'UPDATE public.client_snapshots SET settings_version = settings_version + 1 \
   WHERE account_id = (SELECT min(account_id) FROM public.client_snapshots) \
   RETURNING account_id')"
[[ -n "$stale_snapshot_account" ]] || {
  echo 'could not prepare a stale fixture snapshot for collector recovery check' >&2
  exit 1
}
snapshots_before_restart="$(snapshot_marker)"
docker compose up -d --wait keycloak api frontend simulator snapshots market-collector
wait_for_http "$app_url/" 200 'frontend after Compose stop/start'
wait_for_http "$keycloak_url/realms/aeza/.well-known/openid-configuration" 200 'Keycloak after Compose stop/start'

snapshots_recovered=false
for _ in $(seq 1 60); do
  snapshots_after_restart="$(snapshot_marker)"
  if (( snapshots_after_restart > snapshots_before_restart )); then
    snapshots_recovered=true
    break
  fi
  sleep 1
done
[[ "$snapshots_recovered" == true ]] || {
  echo "snapshot collector did not refresh observations after restart: $snapshots_before_restart → $snapshots_after_restart" >&2
  exit 1
}

postgres_container="$(docker ps --filter 'ancestor=postgres:16' --format '{{.ID}}' | head -n 1)"
[[ -n "$postgres_container" ]] || { echo 'disposable PostgreSQL service container was not found' >&2; exit 1; }
docker restart "$postgres_container" >/dev/null
database_ready=false
for _ in $(seq 1 60); do
  if pg_isready -q -d "$DATABASE_URL"; then database_ready=true; break; fi
  sleep 1
done
[[ "$database_ready" == true ]] || { echo 'PostgreSQL did not recover after its test restart' >&2; exit 1; }
wait_for_http "$app_url/api/v1/openapi.json" 200 'API after PostgreSQL recovery'

after_restart="$(settings_fingerprint)"
[[ "$before_restart" == "$after_restart" ]] || {
  echo "model settings changed across Compose/PostgreSQL restart: $before_restart → $after_restart" >&2
  exit 1
}

echo 'Compose checks passed: migrations, required services, invalid auth startup, stop/start recovery, snapshot collector restart, PostgreSQL restart and settings persistence.'
