# Aéza Historical Model

This crate contains an online behavioral model for one customer at a time. It
adapts the Welford embedding, transition graph, and LCS ideas from the local
`/home/user/Documents/antifrod/backend/src/fraud` project to service and
payment behavior. No training data is required before the first observation.

## Quick start

```sh
cp .env.example .env
# Set DATABASE_URL in .env to the existing PostgreSQL on this host,
# and the OIDC_* variables (see Authentication below).
docker compose up -d --build
```

Open http://127.0.0.1:8080. Compose uses the existing PostgreSQL, applies migrations, loads
`scripts/data.json` when the client tables are empty, and runs the API, daily
client snapshots, the competitor tariff and EUR→RUB collector (daily), the
Market Simulator and the frontend HTTP server. Search interest is collected
only when `TRENDSMCP_API_KEY` is set in `.env`; the Market Intelligence map
includes a Google Trends regional CSV for the map. Ports and credentials
can be overridden in `.env` (see `.env.example`). `docker compose down` stops
the application containers; the existing database remains in place.

### Authentication

The web UI and every `/api` route, including the `/api/client/ws` WebSocket and
the Market Simulator behind the frontend proxy, require a session. Sign-in uses
OpenID Connect (Authorization Code + PKCE) with any provider that publishes
`/.well-known/openid-configuration`:

1. Register a client at the provider with redirect URI
   `<PUBLIC_URL>/api/auth/callback` and post-logout redirect URI `<PUBLIC_URL>/`.
2. Set `OIDC_ISSUER`, `OIDC_CLIENT_ID`, `OIDC_CLIENT_SECRET` (empty for a public
   client) and, if users do not open `http://127.0.0.1:8080`, `PUBLIC_URL` in
   `.env`. Open the UI only at `PUBLIC_URL`: cookies and the WebSocket origin
   check are bound to it.
3. Add the users to `public.admins`. The provider only proves the email
   (`email_verified` must not be false); access is granted to an active row with
   that email, case-insensitively:

   ```sql
   INSERT INTO public.admins (email, name, role)
   VALUES ('analyst@example.com', 'Analyst', 'viewer');
   ```

   `viewer` reads everything; `operator` and `superadmin` may also change the
   Risk Score settings. Setting `"isActive"` to false ends that admin's
   sessions on their next request, and open WebSockets within 30 seconds.

Sessions last `SESSION_HOURS` (12 by default) in an HttpOnly, SameSite=Lax
cookie (`Secure` when `PUBLIC_URL` is https); `public.admin_sessions` stores
only SHA-256 hashes of the tokens. `market_api` refuses to start without the
OIDC settings. For local development and the `scripts/check_*` scripts, which
call the API directly, start it with `AUTH_DISABLED=true`: every request then
acts as a superadmin, so never use it on a reachable host.

#### Check the login locally

`docker-compose.dev-auth.yml` adds a local Keycloak (`127.0.0.1:5556`,
`KEYCLOAK_PORT`) with the realm `aeza` and the client `aeza-intelligence`
from `dev/keycloak/aeza-realm.json`, and points `api` at it in place of the
`OIDC_*` and `AUTH_DISABLED` values from `.env`. Sign-in is a real
email + password form; Keycloak keeps its accounts in the `keycloak-data`
volume.

```sh
docker compose -f docker-compose.yml -f docker-compose.dev-auth.yml up -d
bash scripts/auth_user.sh add me@example.com operator "Имя Фамилия"   # asks for a password
```

Then open http://127.0.0.1:8080: it goes straight to the Keycloak form; sign in
with that email and password. `scripts/auth_user.sh` creates the Keycloak account and the
`public.admins` row together:

```sh
bash scripts/auth_user.sh role me@example.com viewer   # viewer | operator | superadmin
bash scripts/auth_user.sh passwd me@example.com
bash scripts/auth_user.sh disable me@example.com       # enable to restore
bash scripts/auth_user.sh list
```

The Keycloak admin console is at http://127.0.0.1:5556/admin
(`KEYCLOAK_ADMIN` / `KEYCLOAK_ADMIN_PASSWORD`, `admin` / `admin` by default).
An account created there also needs a `public.admins` row (`auth_user.sh role`
does not create one; use `add`, or the SQL above).

`bash scripts/check_auth.sh` starts Keycloak, the API and the frontend, creates
test accounts `auth-*@aeza.test` and goes through the real Authorization Code +
PKCE flow and the Keycloak login form via the frontend proxy. It checks 401
without a session (HTTP, WebSocket, Market Simulator proxy), a wrong password,
`returnTo`, cookie flags, the `forbidden`, `unverified`, `expired` and `denied`
errors, the Origin check for the WebSocket and logout, that a viewer cannot
change Risk Score settings and an operator can, that logout ends the session,
and that deactivation ends it on the next request and closes an open WebSocket
within 30 seconds. `--no-up` checks an already running stack, `--build`
rebuilds the images first, `--cleanup` deactivates the test accounts.

With dev auth on, `scripts/check_api.sh` and the other scripts that call
`127.0.0.1:3000` get 401; `docker compose up -d api` restarts the API with the
settings from `.env` again.

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
default model requires ten complete snapshots spanning 30 days before it makes
confident metric claims. `event_familiarity` uses repeated transitions and their
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

### Check both client models with generated data

`scripts/check_models.sh` uses the Python generator to create 10 fake clients
and 60 dated feature snapshots in a temporary directory. It runs both Rust
models without a database and checks that Snapshot analysis returns all clients,
Historical becomes confident after enough history, and the final behavior
change for the first fixture account is detected.

Run `bash scripts/check_models.sh` for the offline check. The optional
`bash scripts/check_models.sh --persist` adds 10 fixture clients with negative
IDs to the existing PostgreSQL database, writes 60 historical snapshots for
each, checks stored rows and the Rust analysis, Risk Score and Historical HTTP
routes, runs the daily collector twice to check for duplicates, simulates two
UTC days for one fixture account with `--as-of`, then opens
Dashboard, Snapshot and Historical in headless Chromium and compares their
Risk Score with the Rust API.
Existing positive client IDs and identity sequences remain unchanged. The
script refuses to reuse a negative ID owned by a non-fixture record. Dashboard and
Snapshot receive the five-feature Risk Score over the `/api/client/ws`
WebSocket (see [Live channel](#live-channel)); Snapshot reads the
cross-client robust analysis from `GET /api/client/analysis`. Historical reads
persisted daily state and compares recent sequences through
`GET /api/client/historical/{id}/sequence-peers`. Automatically discovered
cross-customer patterns are served by `GET /api/client/historical/patterns`.

### One PostgreSQL database

Set `DATABASE_URL` in `.env` to the existing `aeza_intelligence` database and
run `bash scripts/migrate_database.sh`. This applies the market migrations
(`001`–`004`) and the customer adapter migrations (`100`–`110`) to that same
database. Customer tables live in `public`; market tables live in
`market_intelligence`. The script verifies that all required tables exist and
prints row counts. It does not create or replace a database. Docker Compose
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
loading `.env`. Run `bash scripts/check_api.sh` to print response counts.
Client data is read-only in the frontend; Risk Score settings are stored in
`public.client_settings` and shared by every session. Every route except
`/api/auth/login`, `/callback` and `/logout` requires a session (see
[Authentication](#authentication)).

### Live channel

`GET /api/client/ws` upgrades to a WebSocket carrying JSON messages with a
`type` field. The server sends:

- `snapshot` — `{dataset, settings, risks}` on connect and after a session
  falls behind;
- `dataset` — accounts, services or transactions changed (detected by polling
  `operation_logs` every 2 seconds);
- `scores` — `{changed, removed}`: only the accounts whose Risk Score changed,
  after a data or settings change and at least once a minute, since scores
  depend on the current time;
- `settings` — Risk Score settings changed in any session;
- `ack` / `error` — the reply to a client request (`error.requestId` is `null`
  for an unreadable message).

The client sends `{"type": "updateSettings", "requestId": "1", "settings": {...}}`.
The settings are validated, saved and broadcast to every session together with
the resulting `scores`. The frontend reconnects with exponential backoff and
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

The Rust API listens on `127.0.0.1:3000` and serves the three Market routes
plus client dataset, the `/api/client/ws` live channel, Rust analysis and
historical snapshots. Vite proxies `/api` (including the WebSocket) to that
process. The client dashboard uses the PostgreSQL dataset; Risk Score settings
are stored on the server in `public.client_settings`. Start the existing
local cluster on the host if it is not already running:

```sh
pg_ctl -D .local/postgres -o '-k /tmp -p 55432 -h 127.0.0.1' -l .local/postgres.log start
set -a; . ./.env; set +a
bash scripts/migrate_database.sh
bash scripts/collect_market.sh
bash scripts/collect_trends.sh
cargo run --bin client_snapshot
cargo run --bin market_api
```

In another terminal, run `bash scripts/check_api.sh` to verify response counts,
including the search series per brand. Then start the frontend with
`cd frontend && npm run dev`. On Arch Linux, `docker compose up -d --build api
snapshots` runs the API and daily snapshot scheduler against the same database;
`docker compose --profile collectors up -d trends` additionally schedules
TrendsMCP updates when `TRENDSMCP_API_KEY` is set.

This sandbox cannot connect to the host PostgreSQL or external APIs, so live
row counts and HTTP responses must be checked in the host environment.
