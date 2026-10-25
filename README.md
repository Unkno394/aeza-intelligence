# Aéza Historical Model

This crate contains an online behavioral model for one customer at a time. It
adapts the Welford embedding, transition graph, and LCS ideas from the local
`/home/user/Documents/antifrod/backend/src/fraud` project to service and
payment behavior. No training data is required before the first observation.

## Quick start

```sh
cp .env.example .env
# In .env: DATABASE_URL of the existing PostgreSQL on this host and
# DEFAULT_USER_PASSWORD (16+ characters) for the first sign-in as "admin".
docker compose up -d --build
```

Open https://127.0.0.1:8080 (`server.public_url` of `config.toml`) and sign in
as `admin`. Compose uses the existing PostgreSQL, applies migrations, loads
`scripts/given/data.json` when the client tables are empty, and runs the API, daily
client snapshots, the competitor tariff and EUR→RUB collector (daily), the
Market Simulator and the frontend HTTP server. Search interest is collected
only when `TRENDSMCP_API_KEY` is set in `.env`; the Market Intelligence map
includes a Google Trends regional CSV for the map. `docker compose down` stops
the application containers; the existing database remains in place.

### Configuration

| Where | What |
|---|---|
| `config.toml` | Application settings: public URL, login mode, the default user, OIDC provider, demo stream. Committed; read by `market_api`, `demo_stream`, the frontend and Keycloak. |
| `.env` | Secrets only: `DATABASE_URL`, `DEFAULT_USER_PASSWORD`, `OIDC_CLIENT_SECRET`, the Keycloak passwords, `TRENDSMCP_API_KEY` (see `.env.example`). Not committed. |
| `docker-compose.yml` | Infrastructure: the frontend port 8080, Keycloak on 8081, the `sso` and `demo` profiles. |

Compose mounts `config.toml` read-only into the containers that read it;
after a change run `docker compose up -d api frontend`. Unknown keys and
invalid values stop the API at startup with the key named, instead of falling
back to a default. Outside Docker the binaries read `AEZA_CONFIG`, or
`config.toml` in the working directory. `cargo run --bin aeza_intelligence --
config auth.mode` prints one checked value (without a key, everything).

### Authentication

The web UI and every `/api` route, including the `/api/client/ws` WebSocket and
the Market Simulator behind the frontend proxy, require a session. Access is
granted to active rows of `public.admins`: `viewer` reads everything;
`operator` and `superadmin` may also change the Risk Score settings.
`auth.mode` in `config.toml` chooses how admins sign in:

- **`password`** (the default). The login form checks the email and password
  of a `public.admins` row (Argon2id hash in `password_hash`); no OIDC
  provider or Keycloak is involved. With `[auth.default_user] enabled = true`
  the API creates a `superadmin` with that `login` and `DEFAULT_USER_PASSWORD`
  from `.env` at startup while no admin has a password yet, so the first
  sign-in works on an empty database; later changes to that admin are kept.
  Further admins: `scripts/auth_user.sh add` (below).
- **`oidc`** (Authorization Code + PKCE). The provider only proves the email
  (`email_verified` must not be false); `[auth.oidc]` names it. The default is
  the bundled Keycloak at `http://127.0.0.1:8081`, started with
  `docker compose --profile sso up -d`. Its realm `aeza`, the public PKCE
  client `aeza-intelligence` and the `analyst@aeza.local` account are imported
  from `keycloak/aeza-realm.json` on first start, and `keycloak/start.sh`
  points the client's redirect URIs at `server.public_url` on every start. Set
  `KEYCLOAK_ADMIN_PASSWORD` and `SSO_USER_PASSWORD` in `.env`; the SSO button
  then signs in as `analyst` using `SSO_USER_PASSWORD`. Both are read only
  when the `keycloak` volume is created. To use another provider instead:

  1. Register a client at the provider with redirect URI
     `<public_url>/api/auth/callback` and post-logout redirect URI `<public_url>/`.
  2. Set `issuer` and `client_id` in `[auth.oidc]`, and `OIDC_CLIENT_SECRET`
     in `.env` for a confidential client.
  3. Add the users to `public.admins`; the email is compared case-insensitively:

     ```sql
     INSERT INTO public.admins (email, name, role)
     VALUES ('analyst@example.com', 'Analyst', 'viewer');
     ```
- **`disabled`**: every request acts as a superadmin. Development only; never
  use it on a reachable host.

Open the UI only at `server.public_url`: cookies and the Origin check are
bound to it. Sessions last `server.session_hours` (12 by default) in an
HttpOnly, SameSite=Lax cookie (`Secure` with https). `public.admin_sessions`
stores only SHA-256 hashes of the cookie tokens, so sessions survive an API
restart, and logout or setting `"isActive"` to false ends them on the next
request and closes open WebSockets within 30 seconds.

#### HTTPS

With an `https://` `server.public_url` (the default) the frontend serves HTTPS
on port 8080 and redirects plain HTTP on that port to `public_url`. The
certificate is `certs/tls.crt` with the key `certs/tls.key`. When either is
missing, the frontend generates a self-signed pair at startup for the
`public_url` host plus `localhost` and `127.0.0.1` (valid 825 days), so the
browser shows a warning until the certificate is trusted. To use a real
certificate, put its chain and key there under the same names and run
`docker compose restart frontend`; files written by the container belong to
root. Delete both files to have new ones generated, e.g. after changing the
host in `public_url`. For plain HTTP set `public_url = "http://127.0.0.1:8080"`.

#### Accounts and the login check

`scripts/auth_user.sh` creates the `public.admins` row and the password
together. With `auth.mode = "password"` the password is hashed into that row
through the API image (`aeza_intelligence set-password`, so build the images
first); otherwise it goes to the Keycloak account, kept in the `keycloak`
volume:

```sh
bash scripts/auth_user.sh add me@example.com operator "Имя Фамилия"   # asks for a password
bash scripts/auth_user.sh role me@example.com viewer   # viewer | operator | superadmin
bash scripts/auth_user.sh passwd me@example.com
bash scripts/auth_user.sh disable me@example.com       # enable to restore
bash scripts/auth_user.sh list
```

With OIDC, the Keycloak admin console is at http://127.0.0.1:8081/admin
(`admin` and `KEYCLOAK_ADMIN_PASSWORD`).
An account created there also needs a `public.admins` row (`auth_user.sh role`
does not create one; use `add`, or the SQL above).

`bash scripts/check_auth.sh` signs in end to end through the real Keycloak
form and the frontend proxy: the realm in `keycloak/`, the callback, the
Market Simulator proxy and the Keycloak logout URL. It runs Keycloak, the API
and the frontend with a copy of `config.toml` set to `auth.mode = "oidc"`
(`AEZA_CONFIG_FILE=.local/check-auth.toml`) and switches the API and the
frontend back to `config.toml` afterwards. `--no-up` checks an already running
OIDC stack, `--build` rebuilds the images first, `--cleanup` deactivates the
test account.
The login rules themselves are covered by `cargo test` (see [Tests](#tests)).

## Inputs

- Pass a numeric feature map to `HistoricalModel::observe` for each customer
  snapshot. The PostgreSQL collector records service count, monthly spend,
  spendable balance, payment coverage, upcoming payments and the four Snapshot
  risk shares. Keep each feature's units consistent over time.
- Pass real lifecycle or payment events with `observe_with_event`. Use the
  service ID as `stream_id` so transitions from separate services are not
  joined. An event-only update can use an empty feature map.
- Periodic snapshots without an event must pass `None`. The model cannot infer
  intermediate transitions that were not observed.
- Pass observations for a customer in timestamp order. Timestamps are Unix
  milliseconds.

```rust
use std::collections::BTreeMap;
use aeza_intelligence::historical::{BehaviorEvent, HistoricalModel};

fn main() -> Result<(), Box<dyn std::error::Error>> {
let mut model = HistoricalModel::default();
let features = BTreeMap::from([
    ("server_count".to_owned(), 15.0),
    ("financial_runway_days".to_owned(), 20.0),
]);

let assessment = model.observe_with_event(
    1_780_000_000_000,
    &features,
    Some(BehaviorEvent {
        stream_id: "service-42",
        kind: "prolong_wait",
    }),
);
let _ = assessment.familiarity;

// Check a waiting service even when no new event has arrived.
let pending = model.assess_pending_at(1_780_000_000_000 + 48 * 60 * 60 * 1_000);
let _ = pending;

// Store this compact JSON with the customer's record; restore it later.
let saved = model.to_json()?;
let restored = HistoricalModel::from_json(&saved)?;
let _ = restored;
Ok(())
}
```

## Interpretation

`familiarity` uses snapshot count, history duration, and completeness. The
default model requires ten complete snapshots spanning the configured minimum
history period (30 days by default) before it makes confident metric claims.
The Settings page allows a 7–365 day history period. `event_familiarity` uses repeated transitions and their
history span. Low familiarity prevents the model from making a confident claim.
`is_unusual` means a customer's
current behavior differs from their own history; it is not a churn probability.
The Snapshot Model remains the source for current-state risk, especially when
the historical profile is still sparse.

The embedding keeps lifetime Welford statistics plus short and long profiles
with exponential time decay. Only the decayed profiles decide current
deviation, so an established new habit can become normal. Each metric also
reports `baseline_shift` (recent mean minus long-term mean), which shows the
direction of gradual changes such as declining server count. The graph records
transition frequency and elapsed time. Watched states such as `prolong_wait`
are checked for unusually long waits before a recovery event occurs.

LCS compares each service's recent event sequence with the same customer's prior
windows and with other customers. A bounded-gap frequent-subsequence miner groups
similar sequences and reports only patterns supported by at least two customers;
it does not require a hand-written template. The Historical page also explains
feature and transition anomalies, their personal baseline, and the first
consecutive snapshot or event where the deviation appeared. Similarity describes
sequence shape; it does not establish churn or causation.

The library provides JSON export and import for historical profiles. The
PostgreSQL adapter loads customer datasets and computes current risk features.
`client_snapshot` persists daily model state and assessments, consumes service
and renewal changes from `operation_logs`, and infers expiry events between
daily snapshots. The first run establishes the event-log starting point; old
seed operations are not replayed as customer behavior.

## Market Intelligence

### Continuous synthetic customer stream

The optional `demo-stream` service exercises the live pipeline without touching
real customer rows. It owns three accounts named `demo-stream-*@example.test`
with negative IDs and their nine services. Every 15 seconds it changes one
account. The three profiles emphasize manual renewal and imminent expiry,
insufficient funds near renewal, and `prolong_wait` with a `not_performed`
renewal attempt. Each recovers after the risky phase, so both rising and falling
scores can be observed. The ordinary database triggers append those writes to
`operation_logs`; the API then recalculates Risk Score and pushes the changed
dataset and scores over WebSocket. A `not_performed` transaction is an unpaid
attempt in the fixture, not proof of customer churn.

On first start the service also replays 60 synthetic daily profiles through the
real Historical Model into `client_snapshots`, ending yesterday. This creates a
personal baseline and recent deviations in seconds; it does not pretend the
current database rows were historically observed. The current day remains free
for the normal snapshot collector. The Historical chart shows a separate
**synthetic Risk Score** series for virtual days, calculated with the saved
Snapshot coefficients and the same weighted-severity rule as the live API.
This lets reviewers compare historical scenario risk with behavioral anomaly
without treating either as a measured churn probability. Existing replay data
is kept on restart when the window and settings version still match; otherwise
only the demo clients' past snapshots are rebuilt.

```sh
# Start only the optional generator and its prerequisites; the normal stack may already run.
docker compose --profile demo up -d --build demo-stream
# Stop synthetic writes without removing the fixture or its history.
docker compose stop demo-stream
# One change without Docker, or replay 90 virtual days and exit:
cargo run --bin demo_stream -- --once
cargo run --bin demo_stream -- --replay-days 90 --replay-only
```

`[demo]` of `config.toml` (`interval_seconds` 2–3600, `replay_days` 0 or
31–365, `replay_delay_ms` 0–10000) controls the Compose demo; the command-line
flags of the same names override it. The replay's virtual
days are historical model test inputs; live Risk Score still uses the current
accounts, services and transactions. The generator intentionally has no access
to positive customer IDs.

### Tests

```sh
export DATABASE_URL=postgres://user:password@127.0.0.1:5433/db   # any server where the user may create databases
cargo test                                  # API, models, migrations, snapshot scheduler
(cd market-simulator && cargo test)
(cd frontend && npm test && npm run build)
python3 -m unittest discover -s scripts/given -p 'test_*.py'   # the fixture generator
```

Tests that touch PostgreSQL use `#[sqlx::test]`: each gets a new temporary
database with every migration applied and drops it afterwards, so the data in
`DATABASE_URL` is not touched. GitHub Actions runs the same commands on every
push and pull request (`.github/workflows/ci.yml`).

- `src/bin/market_api/api_tests.rs` starts the real API on a random port. OIDC
  logins go through an in-process provider that signs id_tokens with
  `tests/fixtures/oidc-test-key.pem` (a key used only by the tests) and checks
  PKCE. Covered: 401 without a session, the callback, cookie flags, `returnTo`,
  the `forbidden`, `unverified`, `expired`, `denied` and `failed` errors, two
  logins in one browser, the Origin check, roles over the WebSocket, logout,
  deactivation and expiry, password login with sessions that survive a
  restart, the default user, the client and market routes on
  `scripts/fixtures/data.json`, and that services link only to Aéza tariffs.
- `tests/model_fixture.rs` runs both models over the 60 generated days of
  `scripts/fixtures/model_history.json`: Historical becomes confident, keeps
  its state through JSON and flags the generated change of the first account.
- `src/config.rs`: the committed `config.toml` is valid and holds no secrets,
  and unknown keys or invalid values are reported.
- `tests/migrations.rs`: versions are unique and without gaps, migrate is
  idempotent and creates every table, every migration after 104 rolls back and
  reapplies, and the fixture loads once into empty tables.
- `src/bin/client_snapshot.rs`: one snapshot per account and UTC day across
  restarts and midnight, and a settings change replaces the day's snapshot.

### One PostgreSQL database

Set `DATABASE_URL` in `.env` to the existing `aeza_intelligence` database and
run `cargo run --bin aeza_intelligence -- migrate`. This applies the market migrations
(`001`–`004`) and the customer adapter migrations (`100`–`116`) to that same
database. Customer tables live in `public`; market tables live in
`market_intelligence`. It does not create or replace a database. Docker Compose
uses the same `DATABASE_URL`; see [Quick start](#quick-start).

The client schema compatibility step recognises the former `scripts/001_schema.sql`
tables, checks their required columns and converts their enum fields to text
without deleting rows. SQLx then adds `admins`, `operation_logs`, the feature
functions and `client_snapshots`. A partial or unfamiliar client schema causes
the migration to stop with an explicit error for inspection.

Outside Docker, start the API with `cargo run --bin market_api`. It listens on
`127.0.0.1:3000`; the Vite frontend proxies `/api` there. The API serves
`/api/auth/{login,callback,me,logout}`, `/api/client/ws`, `/api/client/dataset`, `POST /api/client/snapshot`,
`/api/client/analysis`,
`/api/client/historical/{id}`, `/api/client/historical/{id}/sequence-peers`,
`/api/client/historical/patterns`,
and the three `/api/market/*` routes.
For a one-time snapshot run, use `cargo run --bin client_snapshot` after
loading `.env`.
Client data is read-only in the frontend; Risk Score settings are stored in
`public.client_settings`, versioned in `public.client_settings_history`, and
shared by every session. Historical snapshots record the configuration version
used to calculate them. Every route except
`/api/auth/login`, `/callback` and `/logout` requires a session (see
[Authentication](#authentication)).

### Live channel

`GET /api/client/ws` upgrades to a WebSocket carrying JSON messages with a
`type` field. The server sends:

- `snapshot` — `{dataset, settings, settingsMetadata, risks}` on connect and after a session
  falls behind;
- `dataset` — accounts, services or transactions changed (detected by polling
  `operation_logs` every 2 seconds);
- `scores` — `{changed, removed}`: only the accounts whose Risk Score changed,
  after a data or settings change and at least once a minute, since scores
  depend on the current time;
- `settings` — Risk Score settings and their version metadata changed in any session;
- `ack` / `error` — the reply to a client request (`error.requestId` is `null`
  for an unreadable message).

The client sends `{"type": "updateSettings", "requestId": "1", "settings": {...}}`.
The settings are validated, saved with the editor and timestamp, and broadcast
to every session together with the resulting `scores`. The frontend reconnects with exponential backoff and
receives a fresh `snapshot` after each reconnect.

Existing market rows remain in place. The migration runner records applied
versions in `_sqlx_migrations` so repeated runs skip completed migrations.

The [Wayback collector](docs/market-intelligence-wayback.md) and
[PostgreSQL migration](migrations/001_market_intelligence.sql) store current
and archived tariffs in the existing database. A database connection is
required to verify persisted observations.

### Search interest

The active Market Intelligence source is **TrendsMCP** for Aéza, Hetzner,
Contabo, Timeweb Cloud, Selectel, and FirstVDS. A verified terminal request
returned a 14-week Hetzner series; the Rust collector uses the same curl
transport by default and parses both a direct array and a JSON-string
`body` wrapper. Run from the crate root:

```sh
bash scripts/collect_trends.sh
```

Use `cargo run --bin search_interest -- dry-run --keyword Hetzner` to verify
HTTP and JSON without PostgreSQL. `bash scripts/collect_trends.sh` imports
all six brands due for refresh and prints row counts. The scheduler can run
with `cargo run --bin search_interest -- schedule` for weekly updates.
Individual scores are normalized separately, so the page compares changes
within each brand rather than ranking score levels between brands.
The `comparison_group` column records each TrendsMCP keyword as a separate
scale. The paired five-brand Google Trends import uses one shared group;
`GET /api/market/search-interest/comparison` serves only that joint series.

Google Trends Explore CSV import remains an optional path for a joint
comparison and geography. Its automated web endpoint is rate-limited (429).
Export **Interest over time** and **Interest by region** from one comparison
of the same five terms, worldwide, over five years. Import both files with:

```sh
bash scripts/collect_trends.sh --timeline-csv /path/to/multiTimeline.csv --geo-csv /path/to/geoMap.csv
```

The CSV columns must be Aeza, Hetzner, Contabo, Timeweb Cloud, Selectel in
that order. Both CSVs must come from the same comparison window, so their
scores share a scale.

The Market Intelligence map uses the bundled `frontend/src/data/geoMap.csv`
(comparison of five brands, 24 October 2025–24 October 2026) to shade each
country by the selected brand's share. The time-series chart reads the separate
TrendsMCP series. The map geometry and CSV are bundled locally; it does not
require a third-party chart script in the browser.
The optional database importer still accepts paired timeline and geography
exports. Google's official Trends API remains restricted to alpha testers.

The Market Intelligence frontend shows a selected brand's weekly trend,
aggregated monthly values, and 7/30-day changes. The geography map shows
country data after a joint-comparison Google Trends CSV import. A 90-day
free-tier series cannot provide a meaningful yearly chart.

### Currency conversion for competitor tariffs

After importing tariffs, load exchange rates for their observation dates:

```sh
cargo run --bin market_fx
```

For an HTTP and JSON check without PostgreSQL, run
`cargo run --bin market_fx -- --dry-run --date 2025-10-01`.

This applies pending migrations, fetches missing EUR/RUB and USD/RUB
rates from Frankfurter without an API key, and exposes
`market_intelligence.tariff_prices_rub`. The view retains the original
`price_minor` and `currency`, and adds `price_rub_minor`, `rub_rate`, and
`rub_rate_on`. RUB prices use rate 1. Missing historical rates leave the
converted price `NULL`; the importer reports failed dates. Compare prices
only for equivalent billing periods and VAT treatment.

### Tariff identity, quality, and financial journal

`market_intelligence.tariff_products.id` is the stable plan identity across
configuration versions and price observations. `public.services.tariff_product_id`
links an Aéza service to its plan; renewals inherit the link through
`transactions.serviceId`. Existing services are linked only when their names
exactly match an Aéza plan key. The API reports unlinked services so billing
product IDs can be mapped explicitly instead of guessed.

The signed simulator catalog supplies 80 available Aéza offers. Import it with
`cargo run --bin market_import -- aeza-snapshot` or check the signature without
a database using `cargo run --bin market_import -- aeza-snapshot --dry-run`.
Docker's market collector imports the same catalog before scraping competitors.
Normalized CPU, RAM, disk, region, traffic and known price conditions are
stored alongside the source excerpt. Exact price medians require matching
resource, region, traffic and VAT fields; missing fields remain unknown.

`GET /api/market/prices/quality` reports coverage and missing FX rates.
`GET /api/client/data-quality` reports real-client snapshot coverage, missing
plan links and recent behavior events. `GET /api/client/financial-ledger/{id}`
shows performed service charges and increases in `totalReplenished` recorded
as topups. The current billing schema has no explicit refund type; a real
refund feed is required before refunds can be classified.

With PostgreSQL and network access available on the host, run
`bash scripts/check_market_live.sh`. It applies migrations, imports the Aéza
catalog, collects Contabo, fetches FX, then checks the price and quality APIs.

### Run and verify the system

Docker Compose runs everything (see [Quick start](#quick-start)). To work on
the code, run single parts on the host against the same database; they read
`config.toml` from the repository root and the secrets from `.env`:

```sh
set -a; . ./.env; set +a
cargo run --bin aeza_intelligence -- migrate
bash scripts/collect_market.sh          # competitor tariffs and FX once
bash scripts/collect_trends.sh          # search interest, needs TRENDSMCP_API_KEY
cargo run --bin client_snapshot         # one snapshot run
cargo run --bin market_api              # 127.0.0.1:3000; stop the api container first
```

The frontend dev server, `cd frontend && npm run dev`, serves
http://127.0.0.1:5173 and proxies `/api` (including the WebSocket) to
`127.0.0.1:3000`, either container or `cargo run`. The API accepts browser
requests only from `server.public_url`, so the proxy sends that origin in
place of its own. Run `cargo test` before a commit (see [Tests](#tests)).

