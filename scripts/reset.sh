#!/usr/bin/env bash
# Deletes the whole local stack and brings it up from scratch: every container,
# volume and image of docker-compose.yml, the database included. Compose then
# creates the database, applies the migrations and loads scripts/given/data.json.
#
#   bash scripts/reset.sh       asks for confirmation
#   bash scripts/reset.sh -y    without asking
set -euo pipefail

project_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$project_dir"

if [[ "${1:-}" != -y ]]; then
  read -rp "Delete all containers, volumes and the database? [y/N] " answer
  [[ "$answer" == [yY] ]] || exit 1
fi

# The profiles make down include keycloak and demo-stream as well.
docker compose --profile sso --profile demo down --volumes --rmi local --remove-orphans
# keycloak-data is the volume of the old Keycloak 26.0.7, replaced by keycloak.
docker volume rm -f aeza-intelligence_keycloak-data >/dev/null
docker compose up -d --build
docker compose ps -a
