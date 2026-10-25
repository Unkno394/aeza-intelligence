#!/usr/bin/env bash
# Accounts for the bundled Keycloak (docker-compose.yml) and their
# access in public.admins. Keycloak checks the password; public.admins decides
# whether the email may enter and with which role.
#
#   bash scripts/auth_user.sh add <email> <viewer|operator|superadmin> [name]   asks for the password
#   bash scripts/auth_user.sh role <email> <viewer|operator|superadmin>
#   bash scripts/auth_user.sh passwd <email>
#   bash scripts/auth_user.sh disable <email>      ends the sessions on the next request
#   bash scripts/auth_user.sh enable <email>
#   bash scripts/auth_user.sh list
#
# AUTH_USER_PASSWORD sets the password without a prompt. For scripts/check_auth.sh:
# AUTH_USER_UNVERIFIED=1 leaves the email unconfirmed, AUTH_USER_NO_ADMIN=1
# creates only the Keycloak account.
set -euo pipefail

project_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$project_dir"

if [[ -f .env ]]; then
  set -a
  # shellcheck disable=SC1091
  source .env
  set +a
fi
: "${DATABASE_URL:?Set DATABASE_URL in .env}"

usage() { sed -n '6,11p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//' >&2; exit 2; }

kcadm() {
  docker compose exec -T keycloak /opt/keycloak/bin/kcadm.sh "$@" --config /tmp/kcadm.config
}
kc_login() {
  if ! docker compose ps --status running --quiet keycloak | grep -q .; then
    echo 'Keycloak is not running: docker compose up -d keycloak' >&2
    exit 1
  fi
  kcadm config credentials --server http://localhost:8080 --realm master \
    --user "${KEYCLOAK_ADMIN:-admin}" --password "${KEYCLOAK_ADMIN_PASSWORD:-admin}" >/dev/null 2>&1 ||
    { echo 'Keycloak admin login failed (KEYCLOAK_ADMIN / KEYCLOAK_ADMIN_PASSWORD)' >&2; exit 1; }
}
kc_user_id() {
  kcadm get users -r aeza -q "email=$1" -q exact=true --fields id --format csv --noquotes | head -1
}
sql() {
  psql "$DATABASE_URL" -X -q -At -v ON_ERROR_STOP=1 "$@"
}
check_role() {
  [[ "$1" =~ ^(viewer|operator|superadmin)$ ]] || { echo "role must be viewer, operator or superadmin" >&2; exit 2; }
}
read_password() {
  if [[ -n "${AUTH_USER_PASSWORD:-}" ]]; then
    password="$AUTH_USER_PASSWORD"
    return
  fi
  read -rsp 'Password: ' password; echo >&2
  read -rsp 'Repeat: ' repeat; echo >&2
  [[ "$password" == "$repeat" ]] || { echo 'passwords differ' >&2; exit 1; }
  [[ -n "$password" ]] || { echo 'empty password' >&2; exit 1; }
}
set_admin() {
  # $1 email, $2 role or empty to keep, $3 active, $4 name
  sql -v email="$1" -v role="$2" -v active="$3" -v name="$4" <<'SQL'
INSERT INTO public.admins (email, name, role, "isActive")
VALUES (:'email', :'name', coalesce(nullif(:'role', ''), 'viewer'), :'active')
ON CONFLICT (lower(email)) DO UPDATE
  SET role = coalesce(nullif(:'role', ''), public.admins.role),
      "isActive" = EXCLUDED."isActive";
SQL
}

command="${1:-}"
email="${2:-}"
case "$command" in
  add)
    role="${3:-}"
    [[ -n "$email" && -n "$role" ]] || usage
    check_role "$role"
    name="${4:-${email%@*}}"
    read_password
    kc_login
    verified=true
    [[ "${AUTH_USER_UNVERIFIED:-}" == 1 ]] && verified=false
    first="${name%% *}"
    last="${name#* }"
    fields=(-s "email=$email" -s "emailVerified=$verified" -s enabled=true -s "firstName=$first" -s "lastName=${last:-$first}")
    id="$(kc_user_id "$email")"
    if [[ -n "$id" ]]; then
      kcadm update "users/$id" -r aeza "${fields[@]}"
    else
      kcadm create users -r aeza -s "username=$email" "${fields[@]}" >/dev/null 2>&1
    fi
    kcadm set-password -r aeza --username "$email" --new-password "$password"
    if [[ "${AUTH_USER_NO_ADMIN:-}" != 1 ]]; then
      set_admin "$email" "$role" true "$name"
    fi
    echo "$email: Keycloak account ready, role $role"
    ;;
  role)
    role="${3:-}"
    [[ -n "$email" && -n "$role" ]] || usage
    check_role "$role"
    [[ -n "$(sql -v email="$email" -v role="$role" <<'SQL'
UPDATE public.admins SET role = :'role' WHERE lower(email) = lower(:'email') RETURNING id;
SQL
)" ]] || { echo "$email is not in public.admins; use add" >&2; exit 1; }
    echo "$email: role $role (open sessions get it on the next request)"
    ;;
  passwd)
    [[ -n "$email" ]] || usage
    read_password
    kc_login
    kcadm set-password -r aeza --username "$email" --new-password "$password"
    echo "$email: password changed"
    ;;
  disable|enable)
    [[ -n "$email" ]] || usage
    active=$([[ "$command" == enable ]] && echo true || echo false)
    [[ -n "$(sql -v email="$email" -v active="$active" <<'SQL'
UPDATE public.admins SET "isActive" = :'active' WHERE lower(email) = lower(:'email') RETURNING id;
SQL
)" ]] || { echo "$email is not in public.admins" >&2; exit 1; }
    echo "$email: ${command}d"
    ;;
  list)
    psql "$DATABASE_URL" -X -v ON_ERROR_STOP=1 -c \
      'SELECT email, name, role, "isActive" AS active, "lastLoginAt" AS last_login FROM public.admins ORDER BY email'
    ;;
  *)
    usage
    ;;
esac
