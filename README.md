# Aéza Historical Model

This crate contains an online behavioral model for one customer at a time. It
adapts the Welford embedding, transition graph, and LCS ideas from the local
`/home/user/Documents/antifrod/backend/src/fraud` project to service and
payment behavior. No training data is required before the first observation.

## Quick start

```sh
cp .env.example .env
# Set DATABASE_URL in .env to the existing PostgreSQL on this host.
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

## Inputs

- Pass a numeric feature map to `HistoricalModel::observe` for each customer
  snapshot. Useful feature names include `server_count`, `monthly_spend`,
  `financial_runway_days`, and `autorenew_disabled_share`. Keep each feature's
  units consistent over time.
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

`familiarity` uses snapshot count, history duration, and completeness.
`event_familiarity` uses relevant transition history. Low familiarity prevents
the model from making a confident claim. `is_unusual` means a customer's
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

`lcs_similarity` compares event sequences with a supplied pattern. It only
reports sequence similarity; pattern labels and their relationship to churn
must come from separate evidence.

The library provides JSON export and import for historical profiles. The
PostgreSQL adapter loads customer datasets and computes current risk features;
`client_snapshot` persists daily historical assessments in PostgreSQL.

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
script refuses to reuse a negative ID owned by a non-fixture record. The
Snapshot page requests its Risk Score from `POST /api/client/snapshot`; the
Historical page reads persisted daily state.

### One PostgreSQL database

Set `DATABASE_URL` in `.env` to the existing `aeza_intelligence` database and
run `bash scripts/migrate_database.sh`. This applies the market migrations
(`001`–`004`) and the customer adapter migrations (`100`–`108`) to that same
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
`/api/client/dataset`, `/api/client/analysis`,
`/api/client/historical/{id}`, and the three `/api/market/*` routes.
For a one-time snapshot run, use `cargo run --bin client_snapshot` after
loading `.env`. Run `bash scripts/check_api.sh` to print response counts.
Client data is read-only in the frontend; personal model settings are stored
in the browser. The API listens on loopback and has no user authentication,
so do not expose it directly to a public network.

Existing market rows remain in place. The migration runner records applied
versions in `_sqlx_migrations` so repeated runs skip completed migrations.

The [Wayback collector](docs/market-intelligence-wayback.md) and
[PostgreSQL migration](migrations/001_market_intelligence.sql) support four
providers and three tables in the existing database. A database connection is
required to apply the migration and verify persisted observations.

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

This applies `migrations/003_exchange_rates.sql`, fetches missing EUR/RUB
rates from Frankfurter without an API key, and exposes
`market_intelligence.tariff_prices_rub`. The view retains the original
`price_minor` and `currency`, and adds `price_rub_minor`, `rub_rate`, and
`rub_rate_on`. RUB prices use rate 1. Missing historical rates leave the
converted price `NULL`; the importer reports failed dates. Compare prices
only for equivalent billing periods and VAT treatment.

### Run and verify the system

The Rust API listens on `127.0.0.1:3000` and serves the three Market routes
plus client dataset, Rust analysis and historical snapshots. Vite proxies
`/api` to that process. The client dashboard uses the PostgreSQL dataset;
personal Risk Score settings remain in browser storage. Start the existing
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
