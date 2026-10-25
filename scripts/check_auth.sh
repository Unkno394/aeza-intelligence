#!/usr/bin/env bash
# End-to-end OIDC login through the real Keycloak of docker-compose.yml and the
# frontend proxy. The API's own login rules (roles, Origin checks, sessions,
# deactivation) are covered by `cargo test`; this checks what those tests
# cannot: the realm in keycloak/, the Keycloak form and the proxy in front of
# market_api and the Market Simulator.
#
# The check runs the stack with a copy of config.toml set to auth.mode =
# "oidc" (.local/check-auth.toml) and switches api and frontend back to
# config.toml afterwards.
#
#   bash scripts/check_auth.sh            start keycloak + api + frontend, then check
#   bash scripts/check_auth.sh --build    the same, rebuilding the images first
#   bash scripts/check_auth.sh --no-up    check an already running OIDC stack
#   bash scripts/check_auth.sh --cleanup  deactivate the test admin
set -euo pipefail

project_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$project_dir"

up=1
build=()
for arg in "$@"; do
  case "$arg" in
    --build) build=(--build) ;;
    --no-up) up=0 ;;
    --cleanup)
      bash scripts/auth_user.sh disable auth-e2e@aeza.test
      exit 0
      ;;
    *) echo "unknown option: $arg" >&2; exit 2 ;;
  esac
done

email=auth-e2e@aeza.test
password='auth-e2e-Passw0rd'
work="$(mktemp -d)"

if (( up )); then
  mkdir -p .local
  sed -e 's/^mode[[:space:]]*=.*/mode = "oidc"/' config.toml >.local/check-auth.toml
  export AEZA_CONFIG_FILE=.local/check-auth.toml
  trap 'rm -rf "$work"; echo "Switching api and frontend back to config.toml"; AEZA_CONFIG_FILE= docker compose up -d api frontend >/dev/null 2>&1' EXIT
  docker compose --profile sso up -d --wait "${build[@]}" keycloak api frontend
else
  trap 'rm -rf "$work"' EXIT
fi
public_url="$(docker compose run --rm --no-deps -T migrate config server.public_url 2>/dev/null)"
issuer="$(docker compose run --rm --no-deps -T migrate config auth.oidc.issuer 2>/dev/null)"
AUTH_USER_PASSWORD="$password" bash scripts/auth_user.sh add "$email" viewer 'Auth e2e' >/dev/null

failures=0
check() {
  if [[ "$2" == "$3" ]]; then
    echo "ok   $1"
  else
    echo "FAIL $1: expected $3, got $2"
    failures=$((failures + 1))
  fi
}
# status <url> [curl options]: HTTP status, headers in $work/headers, body in $work/body.
status() {
  local url="$1"
  shift
  # -k: the certificate may be the self-signed one of the frontend.
  curl -sk -o "$work/body" -D "$work/headers" -w '%{http_code}' "$@" "$url"
}
location() { sed -n 's/^[Ll]ocation: //p' "$work/headers" | tr -d '\r'; }

for _ in {1..60}; do
  code="$(status "$public_url/api/auth/me" || true)"
  [[ "$code" == 401 ]] && break
  if [[ "$code" == 200 ]]; then
    echo 'market_api answers without a session: auth.mode is "disabled". Run without --no-up.' >&2
    exit 1
  fi
  sleep 2
done

echo 'Without a session'
check '/api/auth/me is 401' "$(status "$public_url/api/auth/me")" 401
check 'Market Simulator proxy is 401' "$(status "$public_url/api/market/v1/catalog")" 401

echo 'Login through the Keycloak form'
app=(-b "$work/app" -c "$work/app")
kc=(-b "$work/kc" -c "$work/kc")
status "$public_url/api/auth/login" "${app[@]}" >/dev/null
authorize="$(location)"
check 'login redirects to the Keycloak realm' "${authorize%%/protocol/*}" "$issuer"
form="$(status "$authorize" "${kc[@]}" >/dev/null; grep -o '<form[^>]*id="kc-form-login"[^>]*' "$work/body" | sed -n 's/.*action="\([^"]*\)".*/\1/p' | sed 's/&amp;/\&/g')"
check 'Keycloak shows its login form' "${form:+found}" found
check 'a wrong password stays on the form' \
  "$(status "$form" "${kc[@]}" --data-urlencode "username=$email" --data-urlencode 'password=wrong')" 200
# The failed attempt renders a new form with a new action URL.
form="$(grep -o '<form[^>]*id="kc-form-login"[^>]*' "$work/body" | sed -n 's/.*action="\([^"]*\)".*/\1/p' | sed 's/&amp;/\&/g')"
status "$form" "${kc[@]}" --data-urlencode "username=$email" --data-urlencode "password=$password" >/dev/null
callback="$(location)"
check 'Keycloak returns to the callback' "${callback%%\?*}" "$public_url/api/auth/callback"
status "$callback" "${app[@]}" >/dev/null
check 'the callback opens the app' "$(location)" /
check '/api/auth/me returns the viewer' \
  "$(status "$public_url/api/auth/me" "${app[@]}"; grep -o '"role":"[a-z]*"' "$work/body")" '200"role":"viewer"'
code="$(status "$public_url/api/market/v1/catalog" "${app[@]}")"
check 'Market Simulator proxy lets the session through' "$([[ "$code" != 401 ]] && echo yes)" yes

echo 'Logout'
status "$public_url/api/auth/logout" "${app[@]}" -X POST -H "Origin: $public_url" >/dev/null
logout_url="$(grep -o '"logoutUrl":"[^"]*"' "$work/body" | cut -d'"' -f4)"
check 'logout returns the Keycloak logout URL' "${logout_url%%/protocol/*}" "$issuer"
check 'the session is gone' "$(status "$public_url/api/auth/me" "${app[@]}")" 401

if (( failures )); then
  echo "$failures check(s) failed" >&2
  exit 1
fi
echo "All checks passed. $email uses the password $password; --cleanup deactivates it."
