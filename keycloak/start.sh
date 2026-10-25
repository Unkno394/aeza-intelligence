#!/bin/bash
# Entrypoint of the bundled Keycloak (docker compose --profile sso). Starts
# Keycloak with the given arguments, then points the aeza-intelligence client
# at server.public_url of config.toml. The realm file is imported only into an
# empty volume, so a later change of public_url would otherwise leave the old
# redirect URIs in place.
set -euo pipefail

config=/etc/aeza/config.toml
public_url=
while IFS= read -r line; do
  if [[ $line =~ ^public_url[[:space:]]*=[[:space:]]*\"([^\"]+)\" ]]; then
    public_url="${BASH_REMATCH[1]%/}"
  fi
done <"$config"
[[ -n "$public_url" ]] || { echo "No server.public_url in $config" >&2; exit 1; }
# Compose interpolates services outside the active profiles too, so the
# passwords are checked here: without the sso profile they need not be set.
[[ -n "${KC_BOOTSTRAP_ADMIN_PASSWORD:-}" && -n "${SSO_USER_PASSWORD:-}" ]] ||
  { echo "Set KEYCLOAK_ADMIN_PASSWORD and SSO_USER_PASSWORD in .env" >&2; exit 1; }
# Read by the realm file on its first import.
export KC_AEZA_PUBLIC_URL="$public_url"

/opt/keycloak/bin/kc.sh "$@" &
keycloak=$!
trap 'kill -TERM "$keycloak"; wait "$keycloak"' TERM INT

kcadm() { /opt/keycloak/bin/kcadm.sh "$@" --config /tmp/kcadm.config; }
until kcadm config credentials --server http://localhost:8080 --realm master \
  --user "$KC_BOOTSTRAP_ADMIN_USERNAME" --password "$KC_BOOTSTRAP_ADMIN_PASSWORD" >/dev/null 2>&1; do
  kill -0 "$keycloak" 2>/dev/null || exit 1
  sleep 2
done
client="$(kcadm get clients -r aeza -q clientId=aeza-intelligence --fields id --format csv --noquotes)"
kcadm update "clients/$client" -r aeza \
  -s "redirectUris=[\"$public_url/api/auth/callback\"]" \
  -s "attributes.\"post.logout.redirect.uris\"=$public_url/"
echo "aeza-intelligence client redirects to $public_url"
# The Compose healthcheck waits for this, not only for Keycloak itself.
touch /tmp/aeza-client-ready
wait "$keycloak"
