#!/usr/bin/env bash
# End-to-end check of the OIDC login against the bundled Keycloak
# (docker-compose.yml). See README "Check the login locally".
#
#   bash scripts/check_auth.sh            start keycloak + api + frontend, create test accounts, run every check
#   bash scripts/check_auth.sh --build    the same, rebuilding the images first
#   bash scripts/check_auth.sh --no-up    run the checks against an already running stack
#   bash scripts/check_auth.sh --cleanup  deactivate the test admins and drop their sessions
set -euo pipefail

project_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$project_dir"

mode=check
up=1
build=()
for arg in "$@"; do
  case "$arg" in
    --build) build=(--build) ;;
    --no-up) up=0 ;;
    --cleanup) mode=cleanup; up=0 ;;
    *) echo "unknown option: $arg" >&2; exit 2 ;;
  esac
done

if [[ -f .env ]]; then
  set -a
  # shellcheck disable=SC1091
  source .env
  set +a
fi
: "${DATABASE_URL:?Set DATABASE_URL in .env}"
export PUBLIC_URL="http://127.0.0.1:${FRONTEND_PORT:-8080}"
export KEYCLOAK_PORT="${KEYCLOAK_PORT:-8081}"
# The API runs against the bundled Keycloak for the check, whatever .env says.
export KEYCLOAK_URL="http://127.0.0.1:$KEYCLOAK_PORT"
export OIDC_ISSUER="$KEYCLOAK_URL/realms/aeza" OIDC_CLIENT_ID=aeza-intelligence OIDC_CLIENT_SECRET= AUTH_DISABLED=false
export CHECK_PASSWORD='auth-check-Passw0rd'
psql_q() { psql "$DATABASE_URL" -X -q -v ON_ERROR_STOP=1 "$@"; }

if [[ "$mode" == cleanup ]]; then
  psql_q <<'SQL'
DELETE FROM public.admin_sessions s USING public.admins a
 WHERE s.admin_id = a.id AND a.email LIKE 'auth-%@aeza.test';
UPDATE public.admins SET "isActive" = false WHERE email LIKE 'auth-%@aeza.test';
SQL
  echo 'Test admins deactivated, their sessions removed.'
  exit 0
fi

if (( up )); then
  docker compose up -d --wait "${build[@]}" keycloak api frontend
fi

# Test accounts, all with the same password. auth-nobody has no public.admins
# row; auth-unverified has an unconfirmed email in Keycloak.
user() { AUTH_USER_PASSWORD="$CHECK_PASSWORD" bash scripts/auth_user.sh add "$@" >/dev/null; }
user auth-viewer@aeza.test viewer 'Auth viewer'
user auth-operator@aeza.test operator 'Auth operator'
user auth-inactive@aeza.test viewer 'Auth inactive'
bash scripts/auth_user.sh disable auth-inactive@aeza.test >/dev/null
AUTH_USER_NO_ADMIN=1 user auth-nobody@aeza.test viewer 'Auth nobody'
AUTH_USER_UNVERIFIED=1 user auth-unverified@aeza.test viewer 'Auth unverified'

python3 - "$mode" <<'PY'
import base64
import html
import http.client
import json
import os
import re
import socket
import struct
import subprocess
import sys
import time
import urllib.parse

MODE = sys.argv[1]
PUBLIC_URL = os.environ['PUBLIC_URL']
FRONT = urllib.parse.urlsplit(PUBLIC_URL)
OIDC_PREFIX = os.environ['OIDC_ISSUER']
PASSWORD = os.environ['CHECK_PASSWORD']
FOREIGN = 'http://evil.example'
failures = []


def check(name, ok, detail=''):
    print(f"{'ok  ' if ok else 'FAIL'} {name}{'' if ok or not detail else f': {detail}'}")
    if not ok:
        failures.append(name)


def request(method, url, cookies=None, body=None, headers=None):
    """One request without following redirects: (status, headers, body, new cookies)."""
    parts = urllib.parse.urlsplit(url)
    connection = http.client.HTTPConnection(parts.hostname, parts.port, timeout=30)
    headers = dict(headers or {})
    if cookies:
        headers['Cookie'] = '; '.join(f'{k}={v}' for k, v in cookies.items())
    if body is not None:
        body = urllib.parse.urlencode(body).encode()
        headers['Content-Type'] = 'application/x-www-form-urlencoded'
    connection.request(method, parts.path + (f'?{parts.query}' if parts.query else ''), body, headers)
    response = connection.getresponse()
    data = response.read()
    set_cookies = response.headers.get_all('Set-Cookie') or []
    jar = {}
    for line in set_cookies:
        key, _, value = line.split(';', 1)[0].partition('=')
        jar[key] = value
    connection.close()
    return response.status, response.headers, data, jar, set_cookies


def api(method, path, cookies=None, **kwargs):
    return request(method, PUBLIC_URL + path, cookies, **kwargs)


def wait_for_api():
    deadline = time.time() + 120
    last = 'no answer'
    while time.time() < deadline:
        try:
            status, _, body, _, _ = api('GET', '/api/auth/me')
            if status == 401:
                return
            if status == 200:
                sys.exit('market_api answers without a session: AUTH_DISABLED is still on. '
                         'Run the check without --no-up to restart it with OIDC.')
            last = f'HTTP {status}'
        except OSError as error:
            last = str(error)
        time.sleep(2)
    sys.exit(f'market_api did not come up with OIDC in 120 s ({last}); see docker compose logs api keycloak')


def start_login(jar, return_to='/'):
    """GET /api/auth/login in a browser holding `jar`; returns the provider URL."""
    status, headers, _, new, _ = api('GET', '/api/auth/login?' + urllib.parse.urlencode({'returnTo': return_to}), jar)
    jar.update(new)
    authorize = headers.get('Location', '')
    assert status in (302, 303, 307) and authorize.startswith(OIDC_PREFIX), (status, authorize)
    return authorize


def finish_login(authorize, jar, email, password=PASSWORD):
    """Signs in on the Keycloak form and follows the callback. Returns (final
    redirect target, Set-Cookie lines of the callback); `jar` is updated."""
    status, _, page, kc, _ = request('GET', authorize)
    form = re.search(r'<form[^>]*id="kc-form-login"[^>]*action="([^"]+)"', page.decode())
    assert status == 200 and form, (status, page[:300])
    status, headers, page, _, _ = request('POST', html.unescape(form.group(1)), kc,
                                          body={'username': email, 'password': password, 'credentialId': ''})
    callback = headers.get('Location', '')
    if status == 200:
        return 'keycloak:rejected', []
    assert status == 302 and callback.startswith(PUBLIC_URL + '/api/auth/callback'), (status, callback)
    status, headers, _, new, set_cookies = request('GET', callback, jar)
    jar.update(new)
    for key in [key for key, value in jar.items() if not value]:
        del jar[key]
    return headers.get('Location', ''), set_cookies


def login(email, password=PASSWORD, return_to='/'):
    """Full Authorization Code + PKCE round trip in a fresh browser.
    Returns (final redirect target, browser cookies, Set-Cookie lines of the callback)."""
    jar = {}
    target, set_cookies = finish_login(start_login(jar, return_to), jar, email, password)
    return target, ({} if target == 'keycloak:rejected' else jar), set_cookies


class Socket:
    """Minimal RFC 6455 client: enough for the JSON text frames of /api/client/ws."""

    def __init__(self, cookies=None, origin=PUBLIC_URL):
        self.sock = socket.create_connection((FRONT.hostname, FRONT.port), timeout=40)
        key = base64.b64encode(os.urandom(16)).decode()
        lines = [f'GET /api/client/ws HTTP/1.1', f'Host: {FRONT.netloc}', 'Upgrade: websocket',
                 'Connection: Upgrade', f'Sec-WebSocket-Key: {key}', 'Sec-WebSocket-Version: 13']
        if origin:
            lines.append(f'Origin: {origin}')
        if cookies:
            lines.append('Cookie: ' + '; '.join(f'{k}={v}' for k, v in cookies.items()))
        self.sock.sendall(('\r\n'.join(lines) + '\r\n\r\n').encode())
        head = b''
        while b'\r\n\r\n' not in head:
            chunk = self.sock.recv(1)
            if not chunk:
                break
            head += chunk
        self.status = int(head.split(b' ', 2)[1]) if head else 0
        self.buffer = b''

    def _read(self, n):
        while len(self.buffer) < n:
            chunk = self.sock.recv(65536)
            if not chunk:
                raise ConnectionError('closed')
            self.buffer += chunk
        data, self.buffer = self.buffer[:n], self.buffer[n:]
        return data

    def recv(self):
        """Next text message as JSON, or None when the server closes the socket."""
        while True:
            try:
                b0, b1 = self._read(2)
            except ConnectionError:
                return None
            length = b1 & 0x7F
            if length == 126:
                length = struct.unpack('!H', self._read(2))[0]
            elif length == 127:
                length = struct.unpack('!Q', self._read(8))[0]
            payload = self._read(length)
            opcode = b0 & 0x0F
            if opcode == 0x8:
                return None
            if opcode == 0x9:
                self._send(0xA, payload)
            elif opcode == 0x1:
                return json.loads(payload)

    def _send(self, opcode, payload):
        mask = os.urandom(4)
        header = bytes([0x80 | opcode])
        if len(payload) < 126:
            header += bytes([0x80 | len(payload)])
        elif len(payload) < 65536:
            header += bytes([0x80 | 126]) + struct.pack('!H', len(payload))
        else:
            header += bytes([0x80 | 127]) + struct.pack('!Q', len(payload))
        self.sock.sendall(header + mask + bytes(b ^ mask[i % 4] for i, b in enumerate(payload)))

    def send(self, message):
        self._send(0x1, json.dumps(message).encode())

    def reply(self, request_id):
        while (message := self.recv()) is not None:
            if message.get('requestId') == request_id:
                return message
        return None

    def close(self):
        self.sock.close()


wait_for_api()

print('Anonymous requests')
for path in ('/api/auth/me', '/api/client/dataset', '/api/market/prices', '/api/market/v1/catalog'):
    status = api('GET', path)[0]
    check(f'GET {path} without a session is 401', status == 401, f'HTTP {status}')
socket_ = Socket()
check('WebSocket without a session is refused', socket_.status == 401, f'HTTP {socket_.status}')
socket_.close()

print('Login: viewer')
target, viewer, set_cookies = login('auth-viewer@aeza.test', return_to='/snapshot?account=1')
check('callback returns to returnTo', target == '/snapshot?account=1', target)
session_line = next((line for line in set_cookies if line.startswith('aeza_session=')), '')
check('session cookie is HttpOnly and SameSite=Lax',
      'HttpOnly' in session_line and 'SameSite=Lax' in session_line, session_line)
check('login cookie is cleared after the callback', not any(k.startswith('aeza_login') for k in viewer), str(viewer))
status, _, body, _, _ = api('GET', '/api/auth/me', viewer)
me = json.loads(body) if status == 200 else {}
check('/api/auth/me returns the viewer', me.get('email') == 'auth-viewer@aeza.test' and me.get('role') == 'viewer', f'HTTP {status} {body[:200]!r}')
for path in ('/api/client/dataset', '/api/market/prices'):
    status = api('GET', path, viewer)[0]
    check(f'GET {path} with a session is 200', status == 200, f'HTTP {status}')
status = api('GET', '/api/market/v1/catalog', viewer)[0]
check('Market Simulator proxy lets a session through', status != 401, f'HTTP {status}')

print('Login: rejected users')
target, jar, _ = login('auth-viewer@aeza.test', 'wrong-password')
check('wrong password is refused by Keycloak', target == 'keycloak:rejected' and not jar, target)
for email, expected in (('auth-nobody@aeza.test', 'forbidden'),
                        ('auth-inactive@aeza.test', 'forbidden'),
                        ('auth-unverified@aeza.test', 'unverified')):
    target, jar, _ = login(email)
    check(f'{email} gets authError={expected}', target == f'/?authError={expected}' and 'aeza_session' not in jar, target)

status, headers, _, jar, _ = api('GET', '/api/auth/login')
state = urllib.parse.parse_qs(urllib.parse.urlsplit(headers['Location']).query)['state'][0]
target = api('GET', '/api/auth/callback?' + urllib.parse.urlencode({'code': 'x', 'state': state}))[1].get('Location')
check('callback from another browser (no login cookie) gets authError=expired', target == '/?authError=expired', target)
target = api('GET', '/api/auth/callback?error=access_denied&state=x', jar)[1].get('Location')
check('provider error gets authError=denied', target == '/?authError=denied', target)
target, _, _ = login('auth-viewer@aeza.test', return_to='//evil.example')
check('returnTo to another site falls back to /', target == '/', target)

print('Login: two tabs of one browser')
browser = {}
first_tab = start_login(browser, '/snapshot')
second_tab = start_login(browser, '/historical')
target, _ = finish_login(first_tab, browser, 'auth-viewer@aeza.test')
check('login finished in the first tab after the second one started', target == '/snapshot', target)
target, _ = finish_login(second_tab, browser, 'auth-viewer@aeza.test')
check('login finished in the second tab as well', target == '/historical', target)
target = api('GET', '/api/auth/callback?code=x&state=stale', browser)[1].get('Location')
check('stale callback in a signed-in browser opens the app', target == '/', target)

print('Login: operator')
target, operator, _ = login('auth-operator@aeza.test')
status, _, body, _, _ = api('GET', '/api/auth/me', operator)
check('/api/auth/me returns the operator', status == 200 and json.loads(body).get('role') == 'operator', f'HTTP {status}')

print('WebSocket')
socket_ = Socket(viewer, origin=FOREIGN)
check('WebSocket from a foreign Origin is 403', socket_.status == 403, f'HTTP {socket_.status}')
socket_.close()
viewer_socket = Socket(viewer)
check('viewer WebSocket upgrades', viewer_socket.status == 101, f'HTTP {viewer_socket.status}')
snapshot = viewer_socket.recv() or {}
check('viewer receives the snapshot', snapshot.get('type') == 'snapshot', str(snapshot.get('type')))
settings = snapshot.get('settings')
viewer_socket.send({'type': 'updateSettings', 'requestId': 'v1', 'settings': settings})
reply = viewer_socket.reply('v1') or {}
check('viewer cannot change Risk Score settings', reply.get('type') == 'error', str(reply))
operator_socket = Socket(operator)
operator_socket.recv()
# The current settings are sent back unchanged, so the shared configuration stays as it was.
operator_socket.send({'type': 'updateSettings', 'requestId': 'o1', 'settings': settings})
reply = operator_socket.reply('o1') or {}
check('operator can change Risk Score settings', reply.get('type') == 'ack', str(reply))
operator_socket.close()

print('Logout')
status = api('POST', '/api/auth/logout', operator, headers={'Origin': FOREIGN})[0]
check('logout from a foreign Origin is 403', status == 403, f'HTTP {status}')
status, _, body, _, set_cookies = api('POST', '/api/auth/logout', operator, headers={'Origin': PUBLIC_URL})
logout_url = json.loads(body).get('logoutUrl') if status == 200 else None
check('logout returns the provider logout URL', bool(logout_url) and logout_url.startswith(OIDC_PREFIX), f'HTTP {status} {logout_url}')
check('logout clears the session cookie', any(line.startswith('aeza_session=;') for line in set_cookies), str(set_cookies))
status = api('GET', '/api/auth/me', operator)[0]
check('old session cookie is rejected after logout', status == 401, f'HTTP {status}')

print('Deactivation')
subprocess.run(['bash', 'scripts/auth_user.sh', 'disable', 'auth-viewer@aeza.test'], check=True, stdout=subprocess.DEVNULL)
try:
    status = api('GET', '/api/auth/me', viewer)[0]
    check('deactivated admin is rejected on the next request', status == 401, f'HTTP {status}')
    print('     waiting up to 35 s for the open WebSocket to close...')
    viewer_socket.sock.settimeout(35)
    started = time.time()
    try:
        while viewer_socket.recv() is not None:
            pass
        closed = True
    except socket.timeout:
        closed = False
    check('open WebSocket of a deactivated admin is closed', closed, 'still open after 35 s')
    print(f'     closed after {time.time() - started:.0f} s')
finally:
    viewer_socket.close()
    subprocess.run(['bash', 'scripts/auth_user.sh', 'enable', 'auth-viewer@aeza.test'], check=True, stdout=subprocess.DEVNULL)

if failures:
    sys.exit(f'\n{len(failures)} auth check(s) failed')
print(f'\nAll auth checks passed. Test accounts auth-*@aeza.test use the password {PASSWORD}; '
      'bash scripts/check_auth.sh --cleanup deactivates them.')
PY
