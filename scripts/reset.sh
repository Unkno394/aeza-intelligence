#!/usr/bin/env bash
# Deletes the whole local stack and brings it up from scratch: the Compose
# containers, volumes and images, and the PostgreSQL container with its data.
# The database is recreated from DATABASE_URL in .env, then Compose applies
# the migrations and loads scripts/data.json.
#
#   bash scripts/reset.sh       asks for confirmation
#   bash scripts/reset.sh -y    without asking
set -euo pipefail

project_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$project_dir"

db_container=aeza-intelligence-db-1
db_volume=aeza-intelligence_db-data
db_image=postgres:16

if [[ -f .env ]]; then
  set -a
  # shellcheck disable=SC1091
  source .env
  set +a
fi
: "${DATABASE_URL:?Set DATABASE_URL in .env}"

# postgres://user:password@host:port/database
if [[ ! "$DATABASE_URL" =~ ^postgres(ql)?://([^:@/]+):([^@/]*)@([^:/]+):([0-9]+)/([^?]+) ]]; then
  echo "DATABASE_URL must look like postgres://user:password@host:port/database" >&2
  exit 1
fi
db_user="${BASH_REMATCH[2]}"
db_password="${BASH_REMATCH[3]}"
db_host="${BASH_REMATCH[4]}"
db_port="${BASH_REMATCH[5]}"
db_name="${BASH_REMATCH[6]}"
if [[ "$db_host" != 127.0.0.1 && "$db_host" != localhost ]]; then
  echo "DATABASE_URL points to $db_host; reset.sh only recreates a local database" >&2
  exit 1
fi

if [[ "${1:-}" != -y ]]; then
  read -rp "Delete all containers, volumes and the database $db_name? [y/N] " answer
  [[ "$answer" == [yY] ]] || exit 1
fi

docker compose down --volumes --rmi local --remove-orphans
docker rm -f "$db_container" >/dev/null 2>&1 || true
# keycloak-data is the volume of the old Keycloak 26.0.7, replaced by keycloak.
docker volume rm -f "$db_volume" aeza-intelligence_keycloak-data >/dev/null

docker run -d --name "$db_container" --restart unless-stopped \
  -e POSTGRES_USER="$db_user" -e POSTGRES_PASSWORD="$db_password" -e POSTGRES_DB="$db_name" \
  -p "$db_port:5432" -v "$db_volume:/var/lib/postgresql/data" \
  "$db_image" >/dev/null
echo "Waiting for PostgreSQL on port $db_port..."
for _ in $(seq 60); do
  if docker exec "$db_container" pg_isready -h 127.0.0.1 -U "$db_user" -d "$db_name" >/dev/null 2>&1; then
    break
  fi
  sleep 1
done
docker exec "$db_container" pg_isready -h 127.0.0.1 -U "$db_user" -d "$db_name" >/dev/null ||
  { echo "PostgreSQL did not start; see docker logs $db_container" >&2; exit 1; }

docker compose up -d --build
docker compose ps -a
