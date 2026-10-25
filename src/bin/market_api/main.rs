#[cfg(test)]
mod api_tests;
mod auth;

use std::{
    collections::BTreeMap,
    net::SocketAddr,
    sync::Arc,
    time::{Duration, Instant},
};

use aeza_intelligence::{
    PgAdapter,
    config::Config,
    analysis::{self, Params},
    historical::{HistoricalModel, lcs_similarity},
    model::ClientFeatures,
    sequences::{DiscoveredPattern, discover_patterns},
};
use anyhow::Result;
use axum::{
    Extension, Json, Router,
    extract::{
        FromRef, Path, Query, State,
        ws::{Message, Utf8Bytes, WebSocket, WebSocketUpgrade},
    },
    http::{StatusCode, header},
    middleware,
    response::Response,
    routing::{get, post, put},
};
use chrono::{DateTime, NaiveDate, Utc};
use clap::Parser;
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row, postgres::PgPoolOptions};
use tokio::sync::{Mutex, broadcast};

use crate::auth::{Admin, Auth, Secrets, Session};

/// How often the live channel checks operation_logs for client data changes.
const POLL_INTERVAL: Duration = Duration::from_secs(2);
/// Scores depend on the current time (renewal window), so they are recalculated
/// at least this often even without data changes.
const RESCORE_INTERVAL: Duration = Duration::from_secs(60);
/// Horizon of the expected-loss scenario of /api/v1/analytics/summary.
const EXPECTED_LOSS_HORIZON_DAYS: i64 = 30;
/// Also how often an open WebSocket rechecks its session; shorter in tests so
/// a deactivated admin's socket closes without a 30-second wait.
const PING_INTERVAL: Duration = if cfg!(test) {
    Duration::from_millis(200)
} else {
    Duration::from_secs(30)
};

#[derive(Parser)]
struct Cli {
    #[arg(long, env = "DATABASE_URL", hide_env_values = true)]
    database_url: String,
    #[arg(long, default_value = "127.0.0.1:3000")]
    listen: SocketAddr,
}

#[derive(Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
struct Interest {
    brand_code: String,
    name: String,
    comparison_group: String,
    observed_on: NaiveDate,
    score: f64,
    change_7d: Option<f64>,
    change_30d: Option<f64>,
}

#[derive(Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
struct Price {
    provider: String,
    plan_name: String,
    observed_on: NaiveDate,
    original_minor: i64,
    currency: String,
    billing_period: String,
    price_rub_minor: Option<i64>,
    rub_rate_on: Option<NaiveDate>,
}

#[derive(Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
struct MonitoredTariff {
    provider_code: String,
    provider: String,
    plan_key: String,
    plan_name: String,
    cpu_cores: Option<f64>,
    ram_gib: Option<f64>,
    disk_gib: Option<f64>,
    disk_type: Option<String>,
    region: Option<String>,
    source_kind: String,
    observed_on: NaiveDate,
    effective_at: Option<DateTime<Utc>>,
    retrieved_at: DateTime<Utc>,
    price_rub: Option<f64>,
    previous_price_rub: Option<f64>,
    price_usd: Option<f64>,
    previous_price_usd: Option<f64>,
    price_eur: Option<f64>,
    previous_price_eur: Option<f64>,
    rub_rate_on: Option<NaiveDate>,
    usd_rub_rate: Option<f64>,
    usd_rate_on: Option<NaiveDate>,
    eur_rub_rate: Option<f64>,
    eur_rate_on: Option<NaiveDate>,
    change_percent: Option<f64>,
    change_percent_usd: Option<f64>,
    change_percent_eur: Option<f64>,
    history_points: i64,
    market_median_rub: Option<f64>,
    market_median_usd: Option<f64>,
    market_median_eur: Option<f64>,
    providers_at_median: i64,
    currency: Option<String>,
    original_price: Option<f64>,
    billing_period: Option<String>,
    vat_included: Option<bool>,
    traffic_gib: Option<i64>,
    traffic_unlimited: Option<bool>,
    price_kind: String,
    normalized_details: Option<serde_json::Value>,
    page_details: Option<String>,
    source_url: String,
    parse_status: String,
}

#[derive(Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
struct MarketPriceChange {
    provider_code: String,
    provider: String,
    plan_key: String,
    plan_name: String,
    cpu_cores: Option<f64>,
    ram_gib: Option<f64>,
    disk_gib: Option<f64>,
    region: Option<String>,
    observed_at: DateTime<Utc>,
    retrieved_at: DateTime<Utc>,
    source_url: String,
    source_kind: String,
    previous_price_minor: i64,
    current_price_minor: i64,
    currency: String,
    change_percent: f64,
}

#[derive(Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
struct MarketSourceStatus {
    provider_code: String,
    provider: String,
    website_url: String,
    tariff_page_urls: Vec<String>,
    last_retrieved_at: Option<DateTime<Utc>>,
    last_complete_at: Option<DateTime<Utc>>,
    complete_observations: i64,
    incomplete_observations: i64,
}

#[derive(Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
struct TariffHistoryPoint {
    provider_code: String,
    plan_key: String,
    cpu_cores: Option<f64>,
    ram_gib: Option<f64>,
    disk_gib: Option<f64>,
    disk_type: Option<String>,
    region: Option<String>,
    observed_on: NaiveDate,
    price_rub: f64,
    price_usd: Option<f64>,
    price_eur: Option<f64>,
    usd_rub_rate: Option<f64>,
    usd_rate_on: Option<NaiveDate>,
    eur_rub_rate: Option<f64>,
    eur_rate_on: Option<NaiveDate>,
}

#[derive(Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
struct BenchmarkHistoryPoint {
    month_start: NaiveDate,
    plan_key: String,
    plan_name: String,
    cpu_cores: f64,
    ram_gib: f64,
    disk_gib: f64,
    disk_type: String,
    region: String,
    price_rub: f64,
    price_usd: Option<f64>,
    price_eur: Option<f64>,
    market_median_rub: f64,
    market_median_usd: Option<f64>,
    market_median_eur: Option<f64>,
    provider_median_min_rub: f64,
    provider_median_max_rub: f64,
    provider_median_min_usd: Option<f64>,
    provider_median_max_usd: Option<f64>,
    provider_median_min_eur: Option<f64>,
    provider_median_max_eur: Option<f64>,
    providers_at_median: i64,
    gap_percent: f64,
    gap_percent_usd: Option<f64>,
    gap_percent_eur: Option<f64>,
}

#[derive(Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
struct TariffClientCohort {
    plan_key: String,
    linked_services: i64,
    accounts: i64,
    awaiting_renewal: i64,
    inactive_services: i64,
    performed_renewals: i64,
    pending_renewal_transactions: i64,
}

#[derive(Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
struct PriceBehaviorRow {
    plan_key: String,
    changed_at: DateTime<Utc>,
    previous_observed_at: Option<DateTime<Utc>>,
    previous_minor: i64,
    current_minor: i64,
    currency: String,
    cohort: String,
    period: String,
    services: i64,
    accounts: i64,
    performed_renewals: i64,
    renewal_revenue_minor: i64,
    prolong_wait_entries: i64,
    prolong_wait_services: i64,
    account_topups_cents: i64,
    complete_followup: bool,
    complete_event_coverage: Option<bool>,
}

#[derive(Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
struct InternalPriceImpactRow {
    event_id: String,
    service_id: String,
    account_id: String,
    account_email: Option<String>,
    plan_key: Option<String>,
    plan_name: String,
    changed_at: DateTime<Utc>,
    previous_price_cents: i64,
    current_price_cents: i64,
    next_renewal_at: DateTime<Utc>,
    payment_term: String,
    complete_baseline: bool,
    complete_pretrend: bool,
    complete_followup: bool,
    renewal_due_in_window: bool,
    current_status: Option<String>,
    current_auto_prolong: Option<bool>,
    renewals_before: i64,
    renewals_earlier: i64,
    renewals_after: i64,
    prolong_wait_after: bool,
    auto_renew_disabled_after: bool,
    inactive_after: bool,
    baseline_services_count: Option<i64>,
    followup_services_count: Option<i64>,
    baseline_monthly_spend: Option<f64>,
    followup_monthly_spend: Option<f64>,
    account_spend_before_cents: i64,
    account_spend_earlier_cents: i64,
    account_spend_after_cents: i64,
    control_account_id: Option<String>,
    control_services_before: Option<i64>,
    control_services_after: Option<i64>,
    control_spend_before_cents: Option<i64>,
    control_spend_after_cents: Option<i64>,
    control_renewed_after: Option<bool>,
    control_renewals_before: Option<i64>,
    control_renewals_earlier: Option<i64>,
    control_spend_earlier_cents: Option<i64>,
}

#[derive(Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
struct GeoInterest {
    brand_code: String,
    source: String,
    region_code: String,
    region_name: String,
    latitude: Option<f64>,
    longitude: Option<f64>,
    period_start: NaiveDate,
    period_end: NaiveDate,
    score: f64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ClientAccount {
    id: String,
    name: String,
    email: String,
    balance_cents: i64,
    total_replenished_cents: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ClientService {
    id: String,
    account_id: String,
    tariff_product_id: Option<String>,
    tariff_plan_key: Option<String>,
    name: String,
    region: Option<String>,
    price_cents: i64,
    renewal_price_cents: i64,
    payment_term_seconds: f64,
    renews_at: DateTime<Utc>,
    auto_renew: bool,
    status: &'static str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ClientTransaction {
    id: String,
    account_id: String,
    service_id: String,
    tariff_product_id: Option<String>,
    amount_cents: i64,
    #[serde(rename = "type")]
    kind: &'static str,
    billing_type: String,
    status: &'static str,
    created_at: DateTime<Utc>,
}

/// Public v1 DTOs intentionally preserve the source snapshot vocabulary. The
/// frontend DTOs above may normalize names for presentation, but the contract
/// API must expose the values documented by the task.
#[derive(Clone, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
struct V1Account {
    id: i64,
    email: String,
    balance: i64,
    withdraw_balance: i64,
    total_replenished: i64,
    bonus_balance: i64,
    bonus_state: String,
}

#[derive(Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
struct V1Service {
    id: i64,
    name: String,
    owner_id: i64,
    payment_term: String,
    auto_prolong: bool,
    created_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
    status: String,
    price: i64,
}

#[derive(Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
struct V1Transaction {
    id: i64,
    amount: i32,
    status: String,
    performed_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
    #[serde(rename = "type")]
    operation_type: String,
    service_id: i64,
}

#[derive(Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
struct FinancialLedgerEntry {
    event_id: String,
    account_id: i64,
    service_id: Option<i64>,
    tariff_product_id: Option<i64>,
    occurred_at: DateTime<Utc>,
    event_type: String,
    amount_cents: i64,
    status: String,
    source: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CashflowWindow {
    days: i64,
    scheduled_renewals: i64,
    projected_outflow_cents: i64,
    balance_after_cents: i64,
    deficit_cents: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CashflowForecast {
    account_id: i64,
    as_of: DateTime<Utc>,
    starting_balance_cents: i64,
    included_services: i64,
    unresolved_services: i64,
    windows: Vec<CashflowWindow>,
    assumption: &'static str,
}

#[derive(Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
struct TariffQuality {
    provider_code: String,
    provider: String,
    products: i64,
    configurations: i64,
    core_specs_known: i64,
    regions_known: i64,
    disk_type_known: i64,
    terms_normalized: i64,
    traffic_known: i64,
    special_conditions_known: i64,
    complete_prices: i64,
    foreign_currency_prices: i64,
    missing_fx_prices: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ClientDataset {
    accounts: Vec<ClientAccount>,
    services: Vec<ClientService>,
    transactions: Vec<ClientTransaction>,
    as_of: DateTime<Utc>,
    snapshots: Vec<serde_json::Value>,
}

#[derive(Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
struct HistoryPoint {
    observed_on: NaiveDate,
    settings_version: i64,
    historical_min_days: Option<i64>,
    assessment: serde_json::Value,
    features: serde_json::Value,
}

#[derive(Clone, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
struct SettingsMetadata {
    version: i64,
    updated_at: Option<DateTime<Utc>>,
    updated_by: Option<String>,
}

impl Default for SettingsMetadata {
    fn default() -> Self {
        Self {
            version: 0,
            updated_at: None,
            updated_by: None,
        }
    }
}

#[derive(sqlx::FromRow)]
struct StoredSettings {
    settings: serde_json::Value,
    version: i64,
    updated_at: DateTime<Utc>,
    updated_by: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SequencePeer {
    account_id: i64,
    similarity: f64,
    sequence: Vec<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SnapshotSettings {
    weights: SnapshotWeights,
    medium_threshold: i64,
    high_threshold: i64,
    renewal_window_days: i64,
    /// Minimum history span before the Historical Model makes confident claims.
    #[serde(default = "default_historical_min_days")]
    historical_min_days: i64,
}

fn default_historical_min_days() -> i64 {
    30
}

impl Default for SnapshotSettings {
    fn default() -> Self {
        Self {
            weights: SnapshotWeights {
                payment_coverage: 35.0,
                no_auto_prolong: 15.0,
                prolong_wait: 15.0,
                failed_prolong: 10.0,
                expiring_revenue: 25.0,
                price_risk: 0.0,
            },
            medium_threshold: 35,
            high_threshold: 65,
            renewal_window_days: 14,
            historical_min_days: default_historical_min_days(),
        }
    }
}

impl SnapshotSettings {
    fn is_valid(&self) -> bool {
        let weights = &self.weights;
        let all = [
            weights.payment_coverage,
            weights.no_auto_prolong,
            weights.prolong_wait,
            weights.failed_prolong,
            weights.expiring_revenue,
            weights.price_risk,
        ];
        all.iter()
            .all(|weight| weight.is_finite() && (0.0..=100.0).contains(weight))
            && all[..5].iter().sum::<f64>() > 0.0
            && (1..=90).contains(&self.renewal_window_days)
            && (1..=100).contains(&self.medium_threshold)
            && self.high_threshold > self.medium_threshold
            && self.high_threshold <= 100
            && (7..=365).contains(&self.historical_min_days)
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SnapshotWeights {
    payment_coverage: f64,
    no_auto_prolong: f64,
    prolong_wait: f64,
    failed_prolong: f64,
    expiring_revenue: f64,
    #[serde(default = "default_price_risk_weight")]
    price_risk: f64,
}

fn default_price_risk_weight() -> f64 {
    0.0
}

#[derive(Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct SnapshotRisk {
    account_id: String,
    score: i64,
    level: &'static str,
    factors: Vec<SnapshotFactor>,
    upcoming_cents: i64,
    monthly_cents: i64,
    topups_cents: i64,
    spendable_balance_cents: i64,
    payment_coverage: Option<f64>,
    assessment_status: &'static str,
    available_factors: usize,
    settings_version: i64,
}

#[derive(Serialize, PartialEq)]
struct SnapshotFactor {
    key: &'static str,
    label: &'static str,
    severity: f64,
    points: f64,
    value: Option<f64>,
    available: bool,
    explanation: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    // Before the database, so a configuration mistake is reported first.
    let config = Config::load()?;
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&cli.database_url)
        .await?;
    PgAdapter::from_pool(pool.clone()).migrate().await?;
    let auth = Auth::new(&config, Secrets::from_env(), &pool).await?;
    let (live, _watcher) = Live::start(pool.clone()).await?;
    let app = router(AppState { pool, live, auth });
    let listener = tokio::net::TcpListener::bind(cli.listen).await?;
    eprintln!("market API listening on http://{}", cli.listen);
    axum::serve(listener, app).await?;
    Ok(())
}

fn router(state: AppState) -> Router {
    // Every route added before route_layer requires a session.
    Router::new()
        .route("/api/market/search-interest", get(search_interest))
        .route(
            "/api/market/search-interest/comparison",
            get(search_interest_comparison),
        )
        .route("/api/market/geography", get(geography))
        .route("/api/market/prices", get(prices))
        .route("/api/market/prices/monitor", get(monitored_prices))
        .route("/api/market/prices/changes", get(market_price_changes))
        .route("/api/market/prices/sources", get(market_source_status))
        .route("/api/market/prices/history", get(price_history))
        .route(
            "/api/market/prices/benchmarks/history",
            get(price_benchmark_history),
        )
        .route(
            "/api/market/prices/client-cohorts",
            get(tariff_client_cohorts),
        )
        .route("/api/market/prices/behavior", get(price_behavior))
        .route("/api/market/prices/quality", get(price_quality))
        .route("/api/client/dataset", get(client_dataset))
        .route("/api/client/analysis", get(client_analysis))
        .route("/api/client/financial-ledger/{id}", get(financial_ledger))
        .route("/api/client/cashflow/{id}", get(client_cashflow))
        .route("/api/client/data-quality", get(client_data_quality))
        .route("/api/client/price-impact", get(client_price_impact))
        .route(
            "/api/client/price-impact-demo",
            get(client_price_impact_demo),
        )
        .route("/api/client/snapshot", post(client_snapshot_risks))
        .route("/api/client/historical/{id}", get(client_history))
        .route(
            "/api/client/historical/{id}/sequence-peers",
            get(client_sequence_peers),
        )
        .route(
            "/api/client/historical/patterns",
            get(discovered_sequence_patterns),
        )
        .route("/api/client/ws", get(live_socket))
        .route("/api/auth/me", get(auth::me))
        .route("/api/v1/auth/me", get(auth::me))
        .route("/api/v1/customers", get(v1_customers))
        .route("/api/v1/customers/{id}", get(v1_customer))
        .route("/api/v1/analytics/summary", get(v1_summary))
        .route(
            "/api/v1/model/settings",
            get(v1_settings).patch(v1_patch_settings),
        )
        .route("/api/v1/source/snapshot", put(v1_source_snapshot))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            auth::require_admin,
        ))
        .route("/api/auth/login", get(auth::login).post(auth::password_login))
        .route("/api/auth/method", get(auth::method))
        .route("/api/auth/callback", get(auth::callback))
        .route("/api/auth/logout", post(auth::logout))
        .route("/api/v1/auth/login", post(auth::password_login))
        .route("/api/v1/auth/logout", post(auth::logout))
        .route("/api/v1/docs", get(v1_docs))
        .route("/api/v1/openapi.json", get(v1_openapi))
        .route("/metrics", get(v1_metrics))
        // The task statement spells the endpoint /metics; both answer.
        .route("/metics", get(v1_metrics))
        .with_state(state)
}

#[derive(Clone)]
struct AppState {
    pool: PgPool,
    live: Arc<Live>,
    auth: Arc<Auth>,
}

#[derive(Deserialize)]
struct CustomerQuery {
    search: Option<String>,
    risk: Option<String>,
    sort: Option<String>,
    direction: Option<String>,
    page: Option<usize>,
    limit: Option<usize>,
}

fn validate_customer_query(query: &CustomerQuery) -> Result<&'static str, StatusCode> {
    let sort = query.sort.as_deref().unwrap_or("score");
    let direction = query.direction.as_deref().unwrap_or("asc");
    if !matches!(direction, "asc" | "desc")
        || query.risk.as_deref().is_some_and(|risk| {
            !matches!(
                risk,
                "all" | "high" | "medium" | "low" | "limited" | "insufficient"
            )
        })
        || query.page.unwrap_or(1) == 0
        || !(1..=500).contains(&query.limit.unwrap_or(25))
    {
        return Err(StatusCode::BAD_REQUEST);
    }
    match sort {
        "id" => Ok("id"),
        "email" => Ok("lower(email)"),
        "balance" => Ok("balance"),
        "totalReplenished" => Ok("\"totalReplenished\""),
        "score" => Ok("id"),
        _ => Err(StatusCode::BAD_REQUEST),
    }
}

fn compare_customer_scores(
    left_score: i64,
    left_id: i64,
    right_score: i64,
    right_id: i64,
    direction: &str,
) -> std::cmp::Ordering {
    let scores = if direction == "desc" {
        right_score.cmp(&left_score)
    } else {
        left_score.cmp(&right_score)
    };
    scores.then_with(|| left_id.cmp(&right_id))
}

/// Synthetic rows (demo stream, model fixtures, price scenarios) have negative
/// IDs. The email domain says nothing: the given dataset uses @example.test.
fn visible_customer(id: &str) -> bool {
    id.parse::<i64>().is_ok_and(|id| id >= 0)
}

fn recurring_service_revenue(service: &ClientService) -> i64 {
    if matches!(service.status, "active" | "suspended") {
        service.price_cents
    } else {
        0
    }
}

/// Score-weighted monthly revenue scenario, rounded to the nearest euro cent.
/// The score is an explicit business proxy, not a calibrated churn probability.
fn modeled_expected_loss(monthly_revenue_cents: i64, risk_score: i64) -> i64 {
    let weighted = i128::from(monthly_revenue_cents.max(0))
        .saturating_mul(i128::from(risk_score.clamp(0, 100)));
    ((weighted + 50) / 100).min(i128::from(i64::MAX)) as i64
}

async fn v1_customers(
    State(state): State<AppState>,
    Query(query): Query<CustomerQuery>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let sort = query.sort.as_deref().unwrap_or("score");
    let direction = query.direction.as_deref().unwrap_or("asc");
    let sql_order = validate_customer_query(&query)?;
    let search = query
        .search
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let sql = format!(
        "SELECT id, email, balance, \"withdrawBalance\" AS withdraw_balance, \
         \"totalReplenished\" AS total_replenished, \"bonusBalance\" AS bonus_balance, \
         \"bonusState\" AS bonus_state FROM public.accounts \
         WHERE id >= 0 \
         AND ($1::text IS NULL OR id::text ILIKE '%' || $1 || '%' OR email ILIKE '%' || $1 || '%') \
         ORDER BY {order} {direction}, id {direction}",
        order = sql_order,
        direction = direction.to_ascii_uppercase(),
    );
    let accounts = sqlx::query_as::<_, V1Account>(&sql)
        .bind(search)
        .fetch_all(&state.pool)
        .await
        .map_err(db_error)?;
    let live = state.live.state.lock().await;
    let mut rows: Vec<_> = accounts
        .into_iter()
        .filter_map(|account| {
            live.risks
                .get(&account.id.to_string())
                .map(|risk| (account, risk))
        })
        .filter(|(_, risk)| {
            query
                .risk
                .as_deref()
                .is_none_or(|level| level == risk.level || level == "all")
        })
        .collect();
    if sort == "score" {
        rows.sort_by(|(a, ar), (b, br)| {
            compare_customer_scores(ar.score, a.id, br.score, b.id, direction)
        });
    }
    let total = rows.len();
    let page = query.page.unwrap_or(1);
    let limit = query.limit.unwrap_or(25);
    let items: Vec<_> = rows
        .into_iter()
        .skip(page.saturating_sub(1).saturating_mul(limit))
        .take(limit)
        .map(|(account, risk)| serde_json::json!({"account":account,"risk":risk}))
        .collect();
    Ok(Json(
        serde_json::json!({"items":items,"total":total,"page":page,"limit":limit}),
    ))
}

async fn v1_customer(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let id = id.parse::<i64>().map_err(|_| StatusCode::NOT_FOUND)?;
    let account = sqlx::query_as::<_, V1Account>(
        "SELECT id, email, balance, \"withdrawBalance\" AS withdraw_balance, \
         \"totalReplenished\" AS total_replenished, \"bonusBalance\" AS bonus_balance, \
         \"bonusState\" AS bonus_state FROM public.accounts \
         WHERE id = $1 AND id >= 0",
    )
    .bind(id)
    .fetch_optional(&state.pool)
    .await
    .map_err(db_error)?
    .ok_or(StatusCode::NOT_FOUND)?;
    let services = sqlx::query_as::<_, V1Service>(
        "SELECT id, name, \"ownerId\" AS owner_id, \"paymentTerm\" AS payment_term, \
         \"autoProlong\" AS auto_prolong, \"createdAt\" AS created_at, \
         \"expiresAt\" AS expires_at, status, price FROM public.services \
         WHERE \"ownerId\" = $1 ORDER BY id",
    )
    .bind(id)
    .fetch_all(&state.pool)
    .await
    .map_err(db_error)?;
    let transactions = sqlx::query_as::<_, V1Transaction>(
        "SELECT t.id, t.amount, t.status, t.\"performedAt\" AS performed_at, \
         t.\"createdAt\" AS created_at, t.type AS operation_type, \
         t.\"serviceId\" AS service_id FROM public.transactions t \
         JOIN public.services s ON s.id = t.\"serviceId\" \
         WHERE s.\"ownerId\" = $1 ORDER BY t.id",
    )
    .bind(id)
    .fetch_all(&state.pool)
    .await
    .map_err(db_error)?;
    let live = state.live.state.lock().await;
    let risk = live
        .risks
        .get(&id.to_string())
        .ok_or(StatusCode::NOT_FOUND)?;
    Ok(Json(
        serde_json::json!({"account":account,"services":services,"transactions":transactions,"risk":risk}),
    ))
}

fn summary(live: &LiveState) -> serde_json::Value {
    let mut levels = BTreeMap::<&str, usize>::new();
    let mut revenue_at_risk = 0_i64;
    let mut deficit = 0_i64;
    let mut expected_loss = 0_i64;
    let mut assessed_monthly_revenue = 0_i64;
    let mut unassessed_monthly_revenue = 0_i64;
    let mut unassessed_customers = 0_usize;
    let mut customers = 0_usize;
    for account in &live.dataset.accounts {
        if !visible_customer(&account.id) {
            continue;
        }
        let Some(risk) = live.risks.get(&account.id) else {
            continue;
        };
        customers += 1;
        *levels.entry(risk.level).or_default() += 1;
        deficit = deficit.saturating_add(
            risk.upcoming_cents
                .saturating_sub(risk.spendable_balance_cents)
                .max(0),
        );
        let monthly_revenue = live
            .dataset
            .services
            .iter()
            .filter(|service| service.account_id == account.id)
            .fold(0_i64, |sum, service| {
                sum.saturating_add(recurring_service_revenue(service))
            });
        if risk.assessment_status == "sufficient_data" {
            assessed_monthly_revenue = assessed_monthly_revenue.saturating_add(monthly_revenue);
            expected_loss =
                expected_loss.saturating_add(modeled_expected_loss(monthly_revenue, risk.score));
        } else {
            unassessed_monthly_revenue = unassessed_monthly_revenue.saturating_add(monthly_revenue);
            unassessed_customers += 1;
        }
        if risk.level == "high" {
            revenue_at_risk = revenue_at_risk.saturating_add(monthly_revenue);
        }
    }
    serde_json::json!({
        "customersTotal":customers,
        "customersByRisk":levels,
        "revenueAtRiskEurCents":revenue_at_risk,
        "paymentDeficitEurCents":deficit,
        "expectedLossEurCents":expected_loss,
        "expectedLossHorizonDays":EXPECTED_LOSS_HORIZON_DAYS,
        "expectedLossAssessedMonthlyRevenueEurCents":assessed_monthly_revenue,
        "expectedLossUnassessedMonthlyRevenueEurCents":unassessed_monthly_revenue,
        "expectedLossUnassessedCustomers":unassessed_customers,
        "expectedLossMethod":"risk_score_weighted_monthly_revenue_v1",
        "expectedLossDefinition":"30-day scenario estimate: active and suspended monthly service revenue multiplied by Risk Score / 100 for customers with sufficient data. Risk Score is an uncalibrated proxy, not a measured churn probability. Revenue for customers with incomplete assessments is excluded and reported separately. This is gross revenue, not profit; payment deficit is reported separately.",
        "lastCalculatedAt":live.last_calculated_at,
        "revenueAtRiskDefinition":"Monthly active and suspended service revenue of high-risk customers; not predicted loss"
    })
}

async fn v1_summary(State(state): State<AppState>) -> Json<serde_json::Value> {
    let live = state.live.state.lock().await;
    Json(summary(&live))
}

async fn v1_settings(State(state): State<AppState>) -> Json<serde_json::Value> {
    let live = state.live.state.lock().await;
    Json(serde_json::json!({"settings":live.settings,"metadata":live.settings_metadata}))
}

async fn v1_patch_settings(
    State(state): State<AppState>,
    Extension(session): Extension<Session>,
    Json(settings): Json<SnapshotSettings>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !session.admin.can_edit_settings() {
        return Err(StatusCode::FORBIDDEN);
    }
    let metadata = state
        .live
        .update_settings(&state.pool, settings, &session.admin.email)
        .await
        .map_err(|message| {
            if message == "Некорректные настройки" {
                StatusCode::BAD_REQUEST
            } else {
                StatusCode::INTERNAL_SERVER_ERROR
            }
        })?;
    let live = state.live.state.lock().await;
    Ok(Json(
        serde_json::json!({"settings":live.settings,"metadata":metadata}),
    ))
}

async fn v1_source_snapshot(
    State(state): State<AppState>,
    Extension(session): Extension<Session>,
    Json(dataset): Json<aeza_intelligence::Dataset>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !session.admin.can_edit_settings() {
        return Err(StatusCode::FORBIDDEN);
    }
    if dataset.accounts.is_empty()
        || dataset
            .accounts
            .iter()
            .any(|account| account.id < 0)
        || dataset.services.iter().any(|service| service.id < 0)
        || dataset
            .transactions
            .iter()
            .any(|transaction| transaction.id < 0)
        || dataset.validate().is_err()
    {
        return Err(StatusCode::BAD_REQUEST);
    }
    let adapter = PgAdapter::from_pool(state.pool.clone());
    let report = adapter.replace(&dataset).await.map_err(|error| {
        eprintln!("snapshot replacement failed: {error}");
        StatusCode::BAD_REQUEST
    })?;
    state
        .live
        .refresh(&state.pool, true)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(
        serde_json::json!({"accounts":report.accounts,"services":report.services,"transactions":report.transactions,"rejected":0}),
    ))
}

async fn v1_metrics(
    State(state): State<AppState>,
) -> ([(header::HeaderName, &'static str); 1], String) {
    let live = state.live.state.lock().await;
    let value = summary(&live);
    let mut output = String::new();
    output.push_str("# HELP aeza_expected_loss_30d_eur_cents Score-weighted 30-day gross revenue scenario in euro cents; not a calibrated churn prediction.\n");
    output.push_str("# HELP aeza_expected_loss_assessed_mrr_eur_cents Monthly revenue included in the expected-loss scenario, in euro cents.\n");
    output.push_str("# HELP aeza_expected_loss_unassessed_mrr_eur_cents Monthly revenue excluded because customer risk data is incomplete, in euro cents.\n");
    output.push_str("# HELP aeza_expected_loss_unassessed_customers Visible customers whose risk data is limited or insufficient.\n");
    for (name, number) in [
        (
            "aeza_customers_total",
            value["customersTotal"].as_u64().unwrap_or(0),
        ),
        (
            "aeza_revenue_at_risk_eur_cents",
            value["revenueAtRiskEurCents"].as_u64().unwrap_or(0),
        ),
        (
            "aeza_payment_deficit_eur_cents",
            value["paymentDeficitEurCents"].as_u64().unwrap_or(0),
        ),
        (
            "aeza_expected_loss_30d_eur_cents",
            value["expectedLossEurCents"].as_u64().unwrap_or(0),
        ),
        (
            "aeza_expected_loss_assessed_mrr_eur_cents",
            value["expectedLossAssessedMonthlyRevenueEurCents"]
                .as_u64()
                .unwrap_or(0),
        ),
        (
            "aeza_expected_loss_unassessed_mrr_eur_cents",
            value["expectedLossUnassessedMonthlyRevenueEurCents"]
                .as_u64()
                .unwrap_or(0),
        ),
        (
            "aeza_expected_loss_unassessed_customers",
            value["expectedLossUnassessedCustomers"]
                .as_u64()
                .unwrap_or(0),
        ),
        (
            "aeza_last_calculation_timestamp_seconds",
            live.last_calculated_at.timestamp().max(0) as u64,
        ),
    ] {
        output.push_str(&format!("# TYPE {name} gauge\n{name} {number}\n"));
    }
    for level in ["high", "medium", "low", "limited", "insufficient"] {
        if level == "high" {
            output.push_str("# TYPE aeza_customers_by_risk gauge\n");
        }
        output.push_str(&format!(
            "aeza_customers_by_risk{{level=\"{level}\"}} {}\n",
            value["customersByRisk"][level].as_u64().unwrap_or(0)
        ));
    }
    (
        [(
            header::CONTENT_TYPE,
            "text/plain; version=0.0.4; charset=utf-8",
        )],
        output,
    )
}

async fn v1_openapi() -> Json<serde_json::Value> {
    Json(
        serde_json::from_str(include_str!("../../../docs/openapi-v1.json"))
            .expect("checked OpenAPI document"),
    )
}

async fn v1_docs() -> axum::response::Html<&'static str> {
    axum::response::Html(
        r#"<!doctype html><html><head><title>Aeza API</title><link rel="stylesheet" href="https://unpkg.com/swagger-ui-dist@5/swagger-ui.css"></head><body><div id="swagger-ui"></div><script src="https://unpkg.com/swagger-ui-dist@5/swagger-ui-bundle.js"></script><script>SwaggerUIBundle({url:'/api/v1/openapi.json',dom_id:'#swagger-ui'});</script></body></html>"#,
    )
}

impl FromRef<AppState> for Arc<Auth> {
    fn from_ref(state: &AppState) -> Self {
        state.auth.clone()
    }
}

impl FromRef<AppState> for PgPool {
    fn from_ref(state: &AppState) -> Self {
        state.pool.clone()
    }
}

/// Messages pushed to frontend sessions over /api/client/ws.
#[derive(Serialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
enum ServerMessage<'a> {
    /// Full state, sent on connect and after a session falls behind the broadcast.
    Snapshot {
        dataset: &'a ClientDataset,
        settings: &'a SnapshotSettings,
        settings_metadata: &'a SettingsMetadata,
        risks: Vec<&'a SnapshotRisk>,
    },
    /// Accounts, services or transactions changed.
    Dataset { dataset: &'a ClientDataset },
    Settings {
        settings: &'a SnapshotSettings,
        settings_metadata: &'a SettingsMetadata,
    },
    /// Only the accounts whose Risk Score changed.
    Scores {
        changed: Vec<&'a SnapshotRisk>,
        removed: Vec<&'a str>,
    },
    Ack {
        request_id: &'a str,
        settings_metadata: &'a SettingsMetadata,
    },
    Error {
        request_id: Option<&'a str>,
        message: &'a str,
    },
}

/// Messages a frontend session sends over /api/client/ws.
#[derive(Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
enum ClientMessage {
    UpdateSettings {
        request_id: String,
        settings: SnapshotSettings,
    },
}

fn encode(message: &ServerMessage) -> Utf8Bytes {
    serde_json::to_string(message)
        .expect("server messages always serialize")
        .into()
}

type Risks = BTreeMap<String, SnapshotRisk>;

/// Accounts whose risk differs between `old` and `new`, and accounts gone from `new`.
fn score_changes<'a>(old: &'a Risks, new: &'a Risks) -> (Vec<&'a SnapshotRisk>, Vec<&'a str>) {
    let changed = new
        .values()
        .filter(|risk| old.get(&risk.account_id) != Some(*risk))
        .collect();
    let removed = old
        .keys()
        .filter(|id| !new.contains_key(*id))
        .map(String::as_str)
        .collect();
    (changed, removed)
}

/// Shared client state for every WebSocket session: the latest dataset, the
/// settings and the Risk Score per account. Changes are broadcast as encoded
/// messages.
struct Live {
    events: broadcast::Sender<Utf8Bytes>,
    state: Mutex<LiveState>,
}

struct LiveState {
    settings: SnapshotSettings,
    settings_metadata: SettingsMetadata,
    dataset: ClientDataset,
    risks: Risks,
    /// Latest operation_logs id: every write to the client tables appends a row.
    log_id: Option<i64>,
    last_calculated_at: DateTime<Utc>,
}

impl Live {
    /// Loads the current state and starts the task that keeps it fresh.
    async fn start(pool: PgPool) -> Result<(Arc<Self>, tokio::task::JoinHandle<()>)> {
        let (settings, settings_metadata) = load_settings(&pool).await?;
        let log_id = last_log_id(&pool).await?;
        let dataset = load_dataset(&pool).await?;
        let mut risks = load_risks(&pool, &settings).await?;
        set_settings_version(&mut risks, settings_metadata.version);
        let live = Arc::new(Self {
            events: broadcast::channel(64).0,
            state: Mutex::new(LiveState {
                settings,
                settings_metadata,
                dataset,
                risks,
                log_id,
                last_calculated_at: Utc::now(),
            }),
        });
        let watcher = tokio::spawn(live.clone().watch(pool));
        Ok((live, watcher))
    }

    async fn watch(self: Arc<Self>, pool: PgPool) {
        let mut tick = tokio::time::interval(POLL_INTERVAL);
        let mut rescored = Instant::now();
        loop {
            tick.tick().await;
            match self
                .refresh(&pool, rescored.elapsed() >= RESCORE_INTERVAL)
                .await
            {
                Ok(true) => rescored = Instant::now(),
                Ok(false) => {}
                Err(error) => eprintln!("live refresh failed: {error:#}"),
            }
        }
    }

    /// Reloads the dataset and scores when client data changed (scores alone
    /// when `rescore` is set) and broadcasts what differs. Returns whether
    /// scores were recalculated.
    async fn refresh(&self, pool: &PgPool, rescore: bool) -> Result<bool> {
        let log_id = last_log_id(pool).await?;
        let mut state = self.state.lock().await;
        let changed = log_id != state.log_id;
        if !changed && !rescore {
            return Ok(false);
        }
        if changed {
            state.dataset = load_dataset(pool).await?;
            state.log_id = log_id;
            self.publish(&ServerMessage::Dataset {
                dataset: &state.dataset,
            });
        }
        let mut risks = load_risks(pool, &state.settings).await?;
        set_settings_version(&mut risks, state.settings_metadata.version);
        self.replace_risks(&mut state, risks);
        state.last_calculated_at = Utc::now();
        Ok(true)
    }

    fn replace_risks(&self, state: &mut LiveState, risks: Risks) {
        let (changed, removed) = score_changes(&state.risks, &risks);
        if !changed.is_empty() || !removed.is_empty() {
            self.publish(&ServerMessage::Scores { changed, removed });
        }
        state.risks = risks;
    }

    async fn update_settings(
        &self,
        pool: &PgPool,
        settings: SnapshotSettings,
        updated_by: &str,
    ) -> Result<SettingsMetadata, &'static str> {
        if !settings.is_valid() {
            return Err("Некорректные настройки");
        }
        // Held across the writes so concurrent updates reach the database and
        // the sessions in the same order.
        let mut state = self.state.lock().await;
        let mut risks = load_risks(pool, &settings).await.map_err(|error| {
            eprintln!("risk calculation for new settings failed: {error:#}");
            "Не удалось пересчитать Risk Score"
        })?;
        let settings_metadata =
            save_settings(pool, &settings, updated_by)
                .await
                .map_err(|error| {
                    eprintln!("saving client settings failed: {error}");
                    "Не удалось сохранить настройки"
                })?;
        set_settings_version(&mut risks, settings_metadata.version);
        state.settings = settings;
        state.settings_metadata = settings_metadata;
        self.publish(&ServerMessage::Settings {
            settings: &state.settings,
            settings_metadata: &state.settings_metadata,
        });
        self.replace_risks(&mut state, risks);
        state.last_calculated_at = Utc::now();
        Ok(state.settings_metadata.clone())
    }

    async fn snapshot(&self) -> Utf8Bytes {
        let state = self.state.lock().await;
        encode(&ServerMessage::Snapshot {
            dataset: &state.dataset,
            settings: &state.settings,
            settings_metadata: &state.settings_metadata,
            risks: state.risks.values().collect(),
        })
    }

    async fn handle(&self, pool: &PgPool, admin: &Admin, text: &str) -> Utf8Bytes {
        match serde_json::from_str::<ClientMessage>(text) {
            Ok(ClientMessage::UpdateSettings { request_id, .. }) if !admin.can_edit_settings() => {
                encode(&ServerMessage::Error {
                    request_id: Some(&request_id),
                    message: "Недостаточно прав для изменения настроек",
                })
            }
            Ok(ClientMessage::UpdateSettings {
                request_id,
                settings,
            }) => match self.update_settings(pool, settings, &admin.email).await {
                Ok(settings_metadata) => encode(&ServerMessage::Ack {
                    request_id: &request_id,
                    settings_metadata: &settings_metadata,
                }),
                Err(message) => encode(&ServerMessage::Error {
                    request_id: Some(&request_id),
                    message,
                }),
            },
            Err(_) => encode(&ServerMessage::Error {
                request_id: None,
                message: "Некорректное сообщение",
            }),
        }
    }

    fn publish(&self, message: &ServerMessage) {
        // Fails only when no session is connected.
        let _ = self.events.send(encode(message));
    }
}

async fn live_socket(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
    Extension(session): Extension<Session>,
) -> Response {
    ws.on_upgrade(move |socket| serve_socket(socket, state, session))
}

async fn serve_socket(mut socket: WebSocket, state: AppState, session: Session) {
    // Subscribe before taking the snapshot so no change falls between them.
    let mut events = state.live.events.subscribe();
    if socket
        .send(Message::Text(state.live.snapshot().await))
        .await
        .is_err()
    {
        return;
    }
    let mut ping =
        tokio::time::interval_at(tokio::time::Instant::now() + PING_INTERVAL, PING_INTERVAL);
    loop {
        let reply = tokio::select! {
            event = events.recv() => match event {
                Ok(text) => text,
                Err(broadcast::error::RecvError::Lagged(_)) => state.live.snapshot().await,
                Err(broadcast::error::RecvError::Closed) => break,
            },
            incoming = socket.recv() => match incoming {
                Some(Ok(Message::Text(text))) => {
                    state.live.handle(&state.pool, &session.admin, &text).await
                }
                Some(Err(_)) | None => break,
                // The close reply is sent by the next read, which then returns None.
                Some(Ok(_)) => continue,
            },
            _ = ping.tick() => {
                // Logout, expiry or deactivation also ends open connections.
                if !state.auth.is_live(&state.pool, &session).await {
                    let _ = socket.send(Message::Close(None)).await;
                    break;
                }
                if socket.send(Message::Ping(Default::default())).await.is_err() {
                    break;
                }
                continue;
            }
        };
        if socket.send(Message::Text(reply)).await.is_err() {
            break;
        }
    }
}

async fn load_settings(pool: &PgPool) -> Result<(SnapshotSettings, SettingsMetadata), sqlx::Error> {
    let stored = sqlx::query_as::<_, StoredSettings>(
        "SELECT settings, version, updated_at, updated_by FROM public.client_settings WHERE id",
    )
    .fetch_optional(pool)
    .await?;
    let Some(stored) = stored else {
        return Ok((SnapshotSettings::default(), SettingsMetadata::default()));
    };
    let settings = match serde_json::from_value::<SnapshotSettings>(stored.settings) {
        Ok(settings) if settings.is_valid() => settings,
        _ => {
            eprintln!("stored client settings are invalid, using defaults");
            SnapshotSettings::default()
        }
    };
    Ok((
        settings,
        SettingsMetadata {
            version: stored.version,
            updated_at: Some(stored.updated_at),
            updated_by: Some(stored.updated_by),
        },
    ))
}

async fn save_settings(
    pool: &PgPool,
    settings: &SnapshotSettings,
    updated_by: &str,
) -> Result<SettingsMetadata, sqlx::Error> {
    let mut transaction = pool.begin().await?;
    let metadata = sqlx::query_as::<_, SettingsMetadata>(
        "INSERT INTO public.client_settings (id, settings, version, updated_by) \
         VALUES (true, $1, 1, $2) \
         ON CONFLICT (id) DO UPDATE SET settings = excluded.settings, \
             version = public.client_settings.version + 1, updated_by = excluded.updated_by, \
             updated_at = now() \
         RETURNING version, updated_at, updated_by",
    )
    .bind(serde_json::to_value(settings).expect("settings always serialize"))
    .bind(updated_by)
    .fetch_one(&mut *transaction)
    .await?;
    sqlx::query(
        "INSERT INTO public.client_settings_history (version, settings, updated_at, updated_by) \
         SELECT version, settings, updated_at, updated_by FROM public.client_settings WHERE id \
         ON CONFLICT (version) DO NOTHING",
    )
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;
    Ok(metadata)
}

fn set_settings_version(risks: &mut Risks, version: i64) {
    for risk in risks.values_mut() {
        risk.settings_version = version;
    }
}

async fn last_log_id(pool: &PgPool) -> Result<Option<i64>, sqlx::Error> {
    sqlx::query_scalar("SELECT max(id) FROM public.operation_logs")
        .fetch_one(pool)
        .await
}

async fn client_dataset(State(pool): State<PgPool>) -> Result<Json<ClientDataset>, StatusCode> {
    load_dataset(&pool).await.map(Json).map_err(db_error)
}

async fn load_dataset(pool: &PgPool) -> Result<ClientDataset, sqlx::Error> {
    let account_rows = sqlx::query(
        "SELECT id::bigint AS id, email, balance, \"totalReplenished\" \
         FROM public.accounts ORDER BY id",
    )
    .fetch_all(pool)
    .await?;
    let service_rows = sqlx::query(
        "SELECT s.id::bigint AS id, s.\"ownerId\"::bigint AS owner_id, s.name, \
         s.tariff_product_id::bigint AS tariff_product_id, product.plan_key AS tariff_plan_key, \
         tariff.region AS region, \
         round(price::numeric * 2592000 / public.payment_term_seconds(\"paymentTerm\"::text))::bigint AS monthly_price, \
         price AS renewal_price, public.payment_term_seconds(\"paymentTerm\"::text) AS payment_term_seconds, \
         \"expiresAt\", \"autoProlong\", status::text AS status \
         FROM public.services s \
         LEFT JOIN market_intelligence.tariff_products product ON product.id = s.tariff_product_id \
         LEFT JOIN LATERAL (SELECT t.region FROM market_intelligence.provider_tariffs t \
             WHERE t.product_id = product.id ORDER BY t.first_observed_at DESC LIMIT 1) tariff ON true \
         ORDER BY s.id",
    )
    .fetch_all(pool)
    .await?;
    let transaction_rows = sqlx::query(
        "SELECT t.id::bigint AS id, s.\"ownerId\"::bigint AS owner_id, \
         t.\"serviceId\"::bigint AS service_id, t.amount::bigint AS amount, \
         s.tariff_product_id::bigint AS tariff_product_id, \
         t.type::text AS type, t.status::text AS status, t.\"createdAt\" \
         FROM public.transactions t JOIN public.services s ON s.id = t.\"serviceId\" ORDER BY t.id",
    )
    .fetch_all(pool)
    .await?;
    let accounts = account_rows
        .into_iter()
        .map(|row| {
            let email: String = row.get("email");
            ClientAccount {
                id: row.get::<i64, _>("id").to_string(),
                name: email.clone(),
                email,
                balance_cents: row.get("balance"),
                total_replenished_cents: row.get("totalReplenished"),
            }
        })
        .collect();
    let services = service_rows
        .into_iter()
        .map(|row| {
            let status: String = row.get("status");
            ClientService {
                id: row.get::<i64, _>("id").to_string(),
                account_id: row.get::<i64, _>("owner_id").to_string(),
                tariff_product_id: row
                    .get::<Option<i64>, _>("tariff_product_id")
                    .map(|id| id.to_string()),
                tariff_plan_key: row.get("tariff_plan_key"),
                name: row.get("name"),
                region: row.get("region"),
                price_cents: row.get("monthly_price"),
                renewal_price_cents: row.get("renewal_price"),
                payment_term_seconds: row.get("payment_term_seconds"),
                renews_at: row.get("expiresAt"),
                auto_renew: row.get("autoProlong"),
                status: match status.as_str() {
                    "deleted" => "cancelled",
                    "active" => "active",
                    "blocked" => "blocked",
                    "prolong_wait" => "prolong_wait",
                    _ => "suspended",
                },
            }
        })
        .collect();
    let transactions = transaction_rows
        .into_iter()
        .map(|row| {
            let status: String = row.get("status");
            ClientTransaction {
                id: row.get::<i64, _>("id").to_string(),
                account_id: row.get::<i64, _>("owner_id").to_string(),
                service_id: row.get::<i64, _>("service_id").to_string(),
                tariff_product_id: row
                    .get::<Option<i64>, _>("tariff_product_id")
                    .map(|id| id.to_string()),
                amount_cents: row.get("amount"),
                kind: "payment",
                billing_type: row.get("type"),
                status: if status == "performed" {
                    "completed"
                } else {
                    "unknown"
                },
                created_at: row.get("createdAt"),
            }
        })
        .collect();
    Ok(ClientDataset {
        accounts,
        services,
        transactions,
        as_of: Utc::now(),
        snapshots: Vec::new(),
    })
}

async fn financial_ledger(
    State(pool): State<PgPool>,
    Path(id): Path<i64>,
) -> Result<Json<Vec<FinancialLedgerEntry>>, StatusCode> {
    let rows = sqlx::query_as::<_, FinancialLedgerEntry>(
        "SELECT ledger.event_id, ledger.account_id, ledger.service_id, \
         service.tariff_product_id, ledger.occurred_at, ledger.event_type, \
         ledger.amount_cents, ledger.status, ledger.source \
         FROM public.financial_ledger ledger \
         LEFT JOIN public.services service ON service.id = ledger.service_id \
         WHERE ledger.account_id = $1 ORDER BY ledger.occurred_at DESC LIMIT 1000",
    )
    .bind(id)
    .fetch_all(&pool)
    .await
    .map_err(db_error)?;
    Ok(Json(rows))
}

/// Scenario forecast with no future top-ups and unchanged renewal prices.
async fn client_cashflow(
    State(pool): State<PgPool>,
    Path(id): Path<i64>,
) -> Result<Json<CashflowForecast>, StatusCode> {
    let now = Utc::now();
    let balance: Option<i64> = sqlx::query_scalar(
        "SELECT balance + CASE WHEN \"bonusState\" = 'unlocked' THEN \"bonusBalance\" ELSE 0 END \
         FROM public.accounts WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(&pool)
    .await
    .map_err(db_error)?;
    let Some(starting_balance_cents) = balance else {
        return Err(StatusCode::NOT_FOUND);
    };
    let rows = sqlx::query(
        "SELECT price::bigint AS renewal_cents, \"expiresAt\", \
         public.payment_term_seconds(\"paymentTerm\"::text) AS term_seconds, \
         status::text AS status, \"autoProlong\" AS auto_prolong \
         FROM public.services WHERE \"ownerId\" = $1 AND status <> 'deleted' \
         AND \"createdAt\" <= $2 ORDER BY id",
    )
    .bind(id)
    .bind(now)
    .fetch_all(&pool)
    .await
    .map_err(db_error)?;
    let horizon = now + chrono::Duration::days(90);
    let limits = [7_i64, 30, 60, 90];
    let mut outflows = [0_i64; 4];
    let mut renewals = [0_i64; 4];
    let mut included_services = 0_i64;
    let mut unresolved_services = 0_i64;
    for row in rows {
        let term_seconds: f64 = row.get("term_seconds");
        if !term_seconds.is_finite() || term_seconds <= 0.0 {
            unresolved_services += 1;
            continue;
        }
        let status: String = row.get("status");
        let auto_prolong: bool = row.get("auto_prolong");
        let mut due_at: DateTime<Utc> = row.get("expiresAt");
        if status != "prolong_wait" && !auto_prolong {
            continue;
        }
        if status == "prolong_wait" && due_at < now {
            due_at = now;
        }
        if due_at > horizon {
            continue;
        }
        included_services += 1;
        let amount: i64 = row.get("renewal_cents");
        let term = chrono::Duration::seconds(term_seconds.ceil() as i64);
        let mut due = due_at;
        while due <= horizon {
            let days = (due - now).num_seconds().max(0) as f64 / 86_400.0;
            for (index, limit) in limits.iter().enumerate() {
                if days <= *limit as f64 {
                    outflows[index] = outflows[index].saturating_add(amount);
                    renewals[index] += 1;
                }
            }
            due += term;
        }
    }
    let windows = limits
        .iter()
        .enumerate()
        .map(|(index, days)| {
            let balance_after_cents = starting_balance_cents.saturating_sub(outflows[index]);
            CashflowWindow {
                days: *days,
                scheduled_renewals: renewals[index],
                projected_outflow_cents: outflows[index],
                balance_after_cents,
                deficit_cents: balance_after_cents.saturating_neg().max(0),
            }
        })
        .collect();
    Ok(Json(CashflowForecast {
        account_id: id,
        as_of: now,
        starting_balance_cents,
        included_services,
        unresolved_services,
        windows,
        assumption: "No top-ups; active auto-renew services and current prolong_wait renew at their current listed price and payment term.",
    }))
}

async fn client_data_quality(
    State(pool): State<PgPool>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let row = sqlx::query(
        "SELECT \
         (SELECT count(*) FROM public.accounts WHERE id >= 0) AS real_accounts, \
         (SELECT count(DISTINCT s.account_id) FROM public.client_snapshots s \
          JOIN public.accounts a ON a.id = s.account_id \
          WHERE a.id >= 0 \
            AND s.observed_on = (now() AT TIME ZONE 'UTC')::date) AS today_accounts, \
         (SELECT count(*) FROM (SELECT s.account_id FROM public.client_snapshots s \
          JOIN public.accounts a ON a.id = s.account_id \
          WHERE a.id >= 0 \
          GROUP BY s.account_id \
          HAVING count(*) >= 10 AND max(s.observed_on) - min(s.observed_on) >= 30) ready) AS real_accounts_with_history, \
         (SELECT max(s.observed_on) FROM public.client_snapshots s \
          JOIN public.accounts a ON a.id = s.account_id \
          WHERE a.id >= 0) AS latest_real_snapshot, \
         (SELECT coalesce(jsonb_agg(jsonb_build_object(\
              'serviceName', service_name, 'services', service_count, 'accounts', account_count) \
              ORDER BY service_count DESC), '[]'::jsonb) \
          FROM (SELECT s.name AS service_name, count(*)::bigint AS service_count, \
                       count(DISTINCT s.\"ownerId\")::bigint AS account_count \
                FROM public.services s JOIN public.accounts a ON a.id = s.\"ownerId\" \
                WHERE s.tariff_product_id IS NULL AND a.id >= 0 \
                GROUP BY s.name ORDER BY count(*) DESC LIMIT 100) unlinked) \
          AS unlinked_service_names, \
         (SELECT count(*) FROM public.transactions WHERE type = 'prolong' \
          AND status = 'not_performed') AS ambiguous_prolongs, \
         (SELECT count(*) FROM public.transactions WHERE type = 'prolong' \
          AND status = 'performed') AS confirmed_successful_prolongs, \
         (SELECT count(*) FROM public.services s \
          JOIN public.accounts a ON a.id = s.\"ownerId\" \
          WHERE s.tariff_product_id IS NULL AND a.id >= 0) AS unlinked_services, \
         (SELECT count(*) FROM public.client_snapshots s \
          JOIN public.accounts a ON a.id = s.account_id \
          WHERE a.id >= 0 \
            AND NOT s.model_state ? 'event_log_cursor') AS snapshots_without_event_cursor, \
         (SELECT count(*) FROM public.operation_logs WHERE entity IN ('service', 'transaction') \
          AND \"occurredAt\" >= now() - interval '7 days') AS recent_behavior_events",
    )
    .fetch_one(&pool)
    .await
    .map_err(db_error)?;
    let real_accounts: i64 = row.get("real_accounts");
    let today_accounts: i64 = row.get("today_accounts");
    Ok(Json(serde_json::json!({
        "realAccounts": real_accounts,
        "todayAccounts": today_accounts,
        "missingToday": real_accounts.saturating_sub(today_accounts),
        "realAccountsWithHistory": row.get::<i64, _>("real_accounts_with_history"),
        "latestRealSnapshot": row.get::<Option<NaiveDate>, _>("latest_real_snapshot"),
        "unlinkedServices": row.get::<i64, _>("unlinked_services"),
        "unlinkedServiceNames": row.get::<serde_json::Value, _>("unlinked_service_names"),
        "ambiguousProlongs": row.get::<i64, _>("ambiguous_prolongs"),
        "renewalOutcomes": {
            "confirmedSuccessful": row.get::<i64, _>("confirmed_successful_prolongs"),
            "unknown": row.get::<i64, _>("ambiguous_prolongs"),
            "confirmedFailed": 0,
            "note": "The source schema exposes performed and not_performed only; not_performed is unresolved."
        },
        "snapshotsWithoutEventCursor": row.get::<i64, _>("snapshots_without_event_cursor"),
        "recentBehaviorEvents": row.get::<i64, _>("recent_behavior_events"),
    })))
}

async fn client_analysis(State(pool): State<PgPool>) -> Result<Json<analysis::Report>, StatusCode> {
    let clients = PgAdapter::from_pool(pool)
        .client_features(Utc::now(), "7 days")
        .await
        .map_err(|error| {
            eprintln!("client analysis query failed: {error}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
    Ok(Json(analysis::analyze(
        &clients,
        Params {
            k: 3,
            shrinkage: 0.3,
        },
    )))
}

async fn client_snapshot_risks(
    State(pool): State<PgPool>,
    Json(settings): Json<SnapshotSettings>,
) -> Result<Json<Vec<SnapshotRisk>>, StatusCode> {
    if !settings.is_valid() {
        return Err(StatusCode::BAD_REQUEST);
    }
    let risks = load_risks(&pool, &settings).await.map_err(|error| {
        eprintln!("snapshot risk query failed: {error:#}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    Ok(Json(risks.into_values().collect()))
}

/// Conservative activation gate. Demo accounts and unresolved renewal outcomes
/// never provide evidence for a production Snapshot Risk Score factor.
fn validated_price_severity(
    rows: &[InternalPriceImpactRow],
    now: DateTime<Utc>,
) -> BTreeMap<i64, f64> {
    use std::collections::BTreeSet;
    let cutoff = now - chrono::Duration::days(183);
    let mut latest: BTreeMap<(usize, i64), &InternalPriceImpactRow> = BTreeMap::new();
    for row in rows {
        let Ok(account) = row.account_id.parse::<i64>() else {
            continue;
        };
        if account <= 0
            || row
                .account_email
                .as_deref()
                .unwrap_or("")
                .ends_with("@example.test")
            || !row.complete_baseline
            || !row.complete_pretrend
            || !row.complete_followup
            || !row.renewal_due_in_window
            || row.renewals_before < row.renewals_earlier
            || row.account_spend_before_cents * 5 < row.account_spend_earlier_cents * 4
        {
            continue;
        }
        let period = usize::from(row.changed_at >= cutoff);
        let entry = latest.entry((period, account)).or_insert(row);
        if row.changed_at > entry.changed_at {
            *entry = row;
        }
    }
    #[derive(Default)]
    struct Cohort {
        eligible: usize,
        known: usize,
        adverse: usize,
        matched: usize,
        matched_spend: usize,
        renewed: usize,
        controls_renewed: usize,
        distinct_controls: BTreeSet<String>,
    }
    let mut cohorts: [[Cohort; 3]; 2] =
        std::array::from_fn(|_| std::array::from_fn(|_| Cohort::default()));
    for ((period, _), row) in &latest {
        let pct = (row.current_price_cents as f64 / row.previous_price_cents as f64 - 1.0) * 100.0;
        let band = if pct < 10.0 {
            0
        } else if pct < 25.0 {
            1
        } else {
            2
        };
        let cohort = &mut cohorts[*period][band];
        cohort.eligible += 1;
        if row.renewals_after > 0 {
            cohort.known += 1;
        } else if row.inactive_after || row.auto_renew_disabled_after || row.prolong_wait_after {
            cohort.known += 1;
            cohort.adverse += 1;
        }
        if let Some(control) = &row.control_account_id
            && row.control_renewals_before == Some(row.renewals_before)
            && row
                .control_renewals_earlier
                .is_some_and(|prior| row.control_renewals_before.unwrap_or(0) >= prior)
            && row
                .control_spend_earlier_cents
                .is_some_and(|prior| row.control_spend_before_cents.unwrap_or(0) * 5 >= prior * 4)
        {
            cohort.matched += 1;
            cohort.distinct_controls.insert(control.clone());
            cohort.renewed += usize::from(row.renewals_after > 0);
            cohort.controls_renewed += usize::from(row.control_renewed_after.unwrap_or(false));
            cohort.matched_spend += usize::from(
                row.account_spend_before_cents > 0
                    && row.control_spend_before_cents.unwrap_or(0) > 0,
            );
        }
    }
    let sufficient = cohorts.iter().flatten().all(|c| {
        c.known >= 100
            && c.known as f64 / c.eligible.max(1) as f64 >= 0.8
            && c.matched >= 100
            && c.matched_spend >= 100
            && c.distinct_controls.len() >= 100
    });
    let period_stable = sufficient
        && cohorts.iter().all(|period| {
            let low = &period[0];
            let high = &period[2];
            high.adverse as f64 / high.known as f64 - low.adverse as f64 / low.known as f64 >= 0.05
                && (high.renewed as f64 / high.matched as f64)
                    < (high.controls_renewed as f64 / high.matched as f64)
        });
    if !period_stable {
        return BTreeMap::new();
    }
    let mut spend_values: Vec<f64> = latest
        .values()
        .filter_map(|row| row.baseline_monthly_spend)
        .filter(|value| value.is_finite())
        .collect();
    spend_values.sort_by(f64::total_cmp);
    if spend_values.len() < 2 {
        return BTreeMap::new();
    }
    let median =
        (spend_values[(spend_values.len() - 1) / 2] + spend_values[spend_values.len() / 2]) / 2.0;
    let mut segments: [[Cohort; 2]; 2] =
        std::array::from_fn(|_| std::array::from_fn(|_| Cohort::default()));
    for row in latest.values() {
        let Some(spend) = row.baseline_monthly_spend.filter(|value| value.is_finite()) else {
            continue;
        };
        let segment = usize::from(spend > median);
        let pct = (row.current_price_cents as f64 / row.previous_price_cents as f64 - 1.0) * 100.0;
        let band = if pct < 10.0 {
            0
        } else if pct >= 25.0 {
            1
        } else {
            continue;
        };
        let cohort = &mut segments[segment][band];
        if row.renewals_after > 0 {
            cohort.known += 1;
        } else if row.inactive_after || row.auto_renew_disabled_after || row.prolong_wait_after {
            cohort.known += 1;
            cohort.adverse += 1;
        }
        if let Some(control) = &row.control_account_id
            && row.control_renewals_before == Some(row.renewals_before)
            && row
                .control_renewals_earlier
                .is_some_and(|prior| row.control_renewals_before.unwrap_or(0) >= prior)
            && row
                .control_spend_earlier_cents
                .is_some_and(|prior| row.control_spend_before_cents.unwrap_or(0) * 5 >= prior * 4)
        {
            cohort.matched += 1;
            cohort.distinct_controls.insert(control.clone());
            cohort.renewed += usize::from(row.renewals_after > 0);
            cohort.controls_renewed += usize::from(row.control_renewed_after.unwrap_or(false));
        }
    }
    let segment_stable = segments.iter().all(|segment| {
        let low = &segment[0];
        let high = &segment[1];
        low.known >= 30
            && high.known >= 30
            && low.distinct_controls.len() >= 30
            && high.distinct_controls.len() >= 30
            && high.adverse as f64 / high.known as f64 - low.adverse as f64 / low.known as f64
                >= 0.05
            && (high.renewed as f64 / high.matched as f64)
                < (high.controls_renewed as f64 / high.matched as f64)
    });
    if !segment_stable {
        return BTreeMap::new();
    }
    let mut result = BTreeMap::new();
    for row in rows {
        let Ok(account) = row.account_id.parse::<i64>() else {
            continue;
        };
        if account <= 0
            || row
                .account_email
                .as_deref()
                .unwrap_or("")
                .ends_with("@example.test")
            || row.changed_at < now - chrono::Duration::days(90)
        {
            continue;
        }
        let increase = ((row.current_price_cents as f64 / row.previous_price_cents as f64 - 1.0)
            / 0.25)
            .clamp(0.0, 1.0);
        let outcome = if row.renewals_after > 0 {
            0.0
        } else if row.inactive_after || row.auto_renew_disabled_after || row.prolong_wait_after {
            1.0
        } else {
            0.5
        };
        result
            .entry(account)
            .and_modify(|value: &mut f64| *value = value.max(increase * outcome))
            .or_insert(increase * outcome);
    }
    result
}

/// Validated price severity by account, recomputed at most once a minute.
static PRICE_RISK_CACHE: std::sync::OnceLock<
    tokio::sync::Mutex<Option<(std::time::Instant, BTreeMap<i64, f64>)>>,
> = std::sync::OnceLock::new();

async fn load_validated_price_severity(pool: &PgPool) -> Result<BTreeMap<i64, f64>> {
    let cache = PRICE_RISK_CACHE.get_or_init(|| tokio::sync::Mutex::new(None));
    let mut cached = cache.lock().await;
    if let Some((checked, values)) = cached.as_ref()
        && checked.elapsed() < std::time::Duration::from_secs(60)
    {
        return Ok(values.clone());
    }
    let candidate_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM public.operation_logs log WHERE log.entity = 'service' \
         AND log.action = 'update' AND log.outcome = 'success' \
         AND log.\"occurredAt\" >= now() - interval '365 days' \
         AND (log.after->>'ownerId')::bigint > 0 \
         AND log.before ? 'price' AND log.after ? 'price' \
         AND (log.after->>'price')::bigint > (log.before->>'price')::bigint",
    )
    .fetch_one(pool)
    .await?;
    let values = if candidate_count >= 600 {
        let rows = sqlx::query_as::<_, InternalPriceImpactRow>(include_str!(
            "../../sql/internal_price_impact.sql"
        ))
        .fetch_all(pool)
        .await?;
        validated_price_severity(&rows, Utc::now())
    } else {
        BTreeMap::new()
    };
    *cached = Some((std::time::Instant::now(), values.clone()));
    Ok(values)
}

/// Risk Score of every client at the current moment, keyed by account ID.
async fn load_risks(pool: &PgPool, settings: &SnapshotSettings) -> Result<Risks> {
    let horizon = format!("{} days", settings.renewal_window_days);
    let clients = PgAdapter::from_pool(pool.clone())
        .client_features(Utc::now(), &horizon)
        .await?;
    let replenished_rows = sqlx::query(
        "SELECT id::bigint AS id, \"totalReplenished\" AS total_replenished FROM public.accounts",
    )
    .fetch_all(pool)
    .await?;
    let replenished: BTreeMap<i64, i64> = replenished_rows
        .into_iter()
        .map(|row| (row.get("id"), row.get("total_replenished")))
        .collect();
    let price_severity = if settings.weights.price_risk > 0.0 {
        load_validated_price_severity(pool).await?
    } else {
        BTreeMap::new()
    };
    Ok(clients
        .iter()
        .map(|client| {
            let risk = calculate_snapshot_risk(
                client,
                replenished
                    .get(&client.account_id)
                    .copied()
                    .unwrap_or_default(),
                settings,
                price_severity.get(&client.account_id).copied(),
            );
            (risk.account_id.clone(), risk)
        })
        .collect())
}

fn calculate_snapshot_risk(
    client: &ClientFeatures,
    topups_cents: i64,
    config: &SnapshotSettings,
    price_risk: Option<f64>,
) -> SnapshotRisk {
    let values = [
        (
            "noAutoProlongShare",
            "Услуги без автопродления",
            client.no_auto_prolong_share,
            config.weights.no_auto_prolong,
            "Доля действующих услуг без автопродления.",
        ),
        (
            "prolongWaitShare",
            "Услуги в prolong_wait",
            client.prolong_wait_share,
            config.weights.prolong_wait,
            "Доля услуг, ожидающих продления.",
        ),
        (
            "failedProlongShare",
            "Неуспешные продления",
            client.failed_prolong_share,
            config.weights.failed_prolong,
            "Не оценивается: not_performed не подтверждает окончательный отказ.",
        ),
        (
            "expiringRevenueShare",
            "Выручка по скоро истекающим услугам",
            client.expiring_revenue_share,
            config.weights.expiring_revenue,
            "Доля месячной выручки от услуг, истекающих в окне анализа.",
        ),
        (
            "paymentCoverage",
            "Покрытие ближайших платежей",
            client.payment_coverage,
            config.weights.payment_coverage,
            "Доступный баланс / платежи в ближайшем окне; риск возникает ниже 1.",
        ),
        (
            "priceRisk",
            "Ценовой риск",
            price_risk,
            config.weights.price_risk,
            if config.weights.price_risk == 0.0 {
                "Ценовой фактор отключён в настройках (вес 0)."
            } else {
                "Недостаточно подтверждённых реальных данных, контрольных пар или устойчивости тенденции."
            },
        ),
    ];
    let total = values
        .iter()
        .filter(|(_, _, value, _, _)| value.is_some_and(f64::is_finite))
        .map(|(_, _, _, weight, _)| *weight)
        .sum::<f64>();
    let factors: Vec<_> = values
        .into_iter()
        .map(|(key, label, value, weight, explanation)| {
            let value = value.filter(|value| value.is_finite());
            let available = value.is_some();
            let severity = value.map_or(0.0, |value| match key {
                "paymentCoverage" => (1.0 - value.clamp(0.0, 1.0)).clamp(0.0, 1.0),
                _ => value.clamp(0.0, 1.0),
            });
            let points = if available && total > 0.0 {
                severity * weight / total * 100.0
            } else { 0.0 };
            SnapshotFactor {
                key,
                label,
                severity,
                points,
                value,
                available,
                explanation: if key == "priceRisk" && available {
                    format!("Проверенные реальные когорты; недавнее повышение и последующий исход дали выраженность {:.0}%, вклад {:.1} балла при текущем весе. Продление, неопределённый и негативный исход дают разный вклад.", severity * 100.0, points)
                } else { explanation.to_owned() },
            }
        })
        .collect();
    let score = factors
        .iter()
        .map(|factor| factor.points)
        .sum::<f64>()
        .round()
        .clamp(0.0, 100.0) as i64;
    let available_factors = factors.iter().filter(|factor| factor.available).count();
    let assessment_status = if client.services_count == 0 {
        "insufficient_data"
    } else if available_factors < 3 || client.payment_coverage.is_none() {
        "limited_data"
    } else {
        "sufficient_data"
    };
    let level = match assessment_status {
        "insufficient_data" => "insufficient",
        "limited_data" => "limited",
        _ if score >= config.high_threshold => "high",
        _ if score >= config.medium_threshold => "medium",
        _ => "low",
    };
    SnapshotRisk {
        account_id: client.account_id.to_string(),
        score,
        level,
        factors,
        upcoming_cents: client.upcoming_payments,
        monthly_cents: client.monthly_spend.round() as i64,
        topups_cents,
        spendable_balance_cents: client.spendable_balance,
        payment_coverage: client.payment_coverage,
        assessment_status,
        available_factors,
        settings_version: 0,
    }
}

async fn client_history(
    State(pool): State<PgPool>,
    Path(id): Path<i64>,
) -> Result<Json<Vec<HistoryPoint>>, StatusCode> {
    let rows = sqlx::query_as::<_, HistoryPoint>(
        "SELECT snapshots.observed_on, snapshots.settings_version, \
             NULLIF(history.settings->>'historicalMinDays', '')::bigint AS historical_min_days, \
             snapshots.assessment, snapshots.features \
         FROM public.client_snapshots snapshots \
         LEFT JOIN public.client_settings_history history ON history.version = snapshots.settings_version \
         WHERE snapshots.account_id = $1 ORDER BY snapshots.observed_on DESC LIMIT 365",
    )
    .bind(id)
    .fetch_all(&pool)
    .await
    .map_err(db_error)?;
    Ok(Json(rows))
}

async fn client_sequence_peers(
    State(pool): State<PgPool>,
    Path(id): Path<i64>,
) -> Result<Json<Vec<SequencePeer>>, StatusCode> {
    let rows = sqlx::query(
        "SELECT DISTINCT ON (account_id) account_id, model_state::text AS state \
         FROM public.client_snapshots ORDER BY account_id, observed_on DESC",
    )
    .fetch_all(&pool)
    .await
    .map_err(db_error)?;
    let mut target = None;
    let mut peers = Vec::new();
    for row in rows {
        let account_id: i64 = row.get("account_id");
        let state: String = row.get("state");
        let model = HistoricalModel::from_json(&state).map_err(|error| {
            eprintln!("invalid saved historical model for {account_id}: {error}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
        let sequence: Vec<String> = model.recent_events().map(str::to_owned).collect();
        if account_id == id {
            target = Some(sequence);
        } else {
            peers.push((account_id, sequence));
        }
    }
    let Some(target) = target.filter(|events| !events.is_empty()) else {
        return Ok(Json(Vec::new()));
    };
    let mut similar: Vec<_> = peers
        .into_iter()
        .filter_map(|(account_id, sequence)| {
            let similarity = lcs_similarity(&target, &sequence);
            (similarity > 0.0).then_some(SequencePeer {
                account_id,
                similarity,
                sequence,
            })
        })
        .collect();
    similar.sort_by(|left, right| right.similarity.total_cmp(&left.similarity));
    similar.truncate(5);
    Ok(Json(similar))
}

async fn discovered_sequence_patterns(
    State(pool): State<PgPool>,
) -> Result<Json<Vec<DiscoveredPattern>>, StatusCode> {
    let rows = sqlx::query(
        "SELECT DISTINCT ON (account_id) account_id, model_state::text AS state \
         FROM public.client_snapshots ORDER BY account_id, observed_on DESC",
    )
    .fetch_all(&pool)
    .await
    .map_err(db_error)?;
    let mut client_sequences = Vec::new();
    for row in rows {
        let account_id: i64 = row.get("account_id");
        let state: String = row.get("state");
        let model = HistoricalModel::from_json(&state).map_err(|error| {
            eprintln!("invalid saved historical model for {account_id}: {error}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
        client_sequences.extend(
            model
                .recent_event_sequences()
                .into_iter()
                .map(|sequence| (account_id, sequence)),
        );
    }
    Ok(Json(discover_patterns(&client_sequences, 2, 20)))
}

fn db_error(error: sqlx::Error) -> StatusCode {
    eprintln!("client API query failed: {error}");
    StatusCode::INTERNAL_SERVER_ERROR
}

async fn geography(State(pool): State<PgPool>) -> Result<Json<Vec<GeoInterest>>, StatusCode> {
    let rows = sqlx::query_as::<_, GeoInterest>(
        "SELECT brand_code, source, region_code, region_name, latitude, longitude, \
         period_start, period_end, score::float8 AS score \
         FROM market_intelligence.search_interest_geo \
         WHERE source = 'google trends compare 5y' \
         ORDER BY period_end DESC, brand_code, region_name LIMIT 5000",
    )
    .fetch_all(&pool)
    .await
    .map_err(|error| {
        eprintln!("geography query failed: {error}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    Ok(Json(rows))
}

async fn search_interest(State(pool): State<PgPool>) -> Result<Json<Vec<Interest>>, StatusCode> {
    Ok(Json(load_interest(&pool, "google search").await?))
}

async fn search_interest_comparison(
    State(pool): State<PgPool>,
) -> Result<Json<Vec<Interest>>, StatusCode> {
    Ok(Json(
        load_interest(&pool, "google trends compare 5y").await?,
    ))
}

async fn load_interest(pool: &PgPool, source: &str) -> Result<Vec<Interest>, StatusCode> {
    let rows = sqlx::query_as::<_, Interest>(
        "SELECT i.brand_code, b.name, i.comparison_group, i.observed_on, i.score::float8 AS score, \
         c.change_7d::float8 AS change_7d, c.change_30d::float8 AS change_30d \
         FROM market_intelligence.search_interest i \
         JOIN market_intelligence.search_brands b ON b.code = i.brand_code \
         LEFT JOIN market_intelligence.search_interest_changes c \
           ON c.brand_code = i.brand_code AND c.source = i.source \
           AND c.comparison_group = i.comparison_group \
           AND c.observed_on = i.observed_on \
         WHERE i.source = $1 \
         ORDER BY b.name, i.observed_on",
    )
    .bind(source)
    .fetch_all(pool)
    .await
    .map_err(|error| {
        eprintln!("search-interest query failed: {error}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    Ok(rows)
}

async fn price_quality(State(pool): State<PgPool>) -> Result<Json<Vec<TariffQuality>>, StatusCode> {
    let rows = sqlx::query_as::<_, TariffQuality>(
        "SELECT provider_code, provider, products, configurations, core_specs_known, \
         regions_known, disk_type_known, terms_normalized, traffic_known, \
         special_conditions_known, complete_prices, \
         foreign_currency_prices, missing_fx_prices \
         FROM market_intelligence.tariff_data_quality ORDER BY provider_code",
    )
    .fetch_all(&pool)
    .await
    .map_err(db_error)?;
    Ok(Json(rows))
}

async fn market_price_changes(
    State(pool): State<PgPool>,
) -> Result<Json<Vec<MarketPriceChange>>, StatusCode> {
    let rows =
        sqlx::query_as::<_, MarketPriceChange>(include_str!("../../sql/market_price_changes.sql"))
            .fetch_all(&pool)
            .await
            .map_err(db_error)?;
    Ok(Json(rows))
}

async fn market_source_status(
    State(pool): State<PgPool>,
) -> Result<Json<Vec<MarketSourceStatus>>, StatusCode> {
    let rows = sqlx::query_as::<_, MarketSourceStatus>(
        "SELECT provider.code AS provider_code, provider.name AS provider, \
         provider.website_url, provider.tariff_page_urls, \
         max(price.retrieved_at) AS last_retrieved_at, \
         max(price.retrieved_at) FILTER (WHERE price.parse_status = 'complete') AS last_complete_at, \
         count(price.id) FILTER (WHERE price.parse_status = 'complete')::bigint AS complete_observations, \
         count(price.id) FILTER (WHERE price.parse_status = 'incomplete')::bigint AS incomplete_observations \
         FROM market_intelligence.providers provider \
         LEFT JOIN market_intelligence.provider_tariffs tariff ON tariff.provider_id = provider.id \
         LEFT JOIN market_intelligence.tariff_prices price ON price.provider_tariff_id = tariff.id \
         GROUP BY provider.id, provider.code, provider.name, provider.website_url, provider.tariff_page_urls \
         ORDER BY provider.name",
    )
    .fetch_all(&pool)
    .await
    .map_err(db_error)?;
    Ok(Json(rows))
}

async fn prices(State(pool): State<PgPool>) -> Result<Json<Vec<Price>>, StatusCode> {
    let rows = sqlx::query_as::<_, Price>(
        "SELECT DISTINCT ON (p.provider_tariff_id) providers.name AS provider, \
         p.plan_name, (p.observed_at AT TIME ZONE 'UTC')::date AS observed_on, \
         p.price_minor AS original_minor, p.currency::text AS currency, \
         p.billing_period, p.price_rub_minor, p.rub_rate_on \
         FROM market_intelligence.tariff_prices_rub p \
         JOIN market_intelligence.providers ON providers.id = p.provider_id \
         WHERE p.parse_status = 'complete' \
         ORDER BY p.provider_tariff_id, p.observed_at DESC LIMIT 500",
    )
    .fetch_all(&pool)
    .await
    .map_err(|error| {
        eprintln!("prices query failed: {error}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    Ok(Json(rows))
}

/// Current scraped tariffs plus their prior observed price for change tracking.
/// The data comes from the page parsers' PostgreSQL history, not the simulator fixture catalog.
async fn monitored_prices(
    State(pool): State<PgPool>,
) -> Result<Json<Vec<MonitoredTariff>>, StatusCode> {
    let rows = sqlx::query_as::<_, MonitoredTariff>(
        r#"WITH observed AS (
             SELECT providers.code AS provider_code, providers.name AS provider,
                    t.plan_key, t.plan_name, t.cpu_cores::float8 AS cpu_cores,
                    t.ram_mib::float8 / 1024.0 AS ram_gib,
                    t.disk_gib::float8 AS disk_gib, t.disk_type, t.region,
                    p.source_kind, p.effective_at, p.retrieved_at,
                    (p.observed_at AT TIME ZONE 'UTC')::date AS observed_on,
                    p.price_rub_minor::float8 / 100.0 AS price_rub,
                    lag(p.price_rub_minor::float8 / 100.0) OVER w AS previous_price_rub,
                    CASE WHEN p.currency = 'USD' THEN p.price_minor::float8 / 100.0
                         WHEN usd_fx.rate > 0 THEN p.price_rub_minor::float8 / 100.0 / usd_fx.rate::float8
                         ELSE NULL END AS price_usd,
                    lag(CASE WHEN p.currency = 'USD' THEN p.price_minor::float8 / 100.0
                         WHEN usd_fx.rate > 0 THEN p.price_rub_minor::float8 / 100.0 / usd_fx.rate::float8
                         ELSE NULL END) OVER w AS previous_price_usd,
                    CASE WHEN p.currency = 'EUR' THEN p.price_minor::float8 / 100.0
                         WHEN eur_fx.rate > 0 THEN p.price_rub_minor::float8 / 100.0 / eur_fx.rate::float8
                         ELSE NULL END AS price_eur,
                    lag(CASE WHEN p.currency = 'EUR' THEN p.price_minor::float8 / 100.0
                         WHEN eur_fx.rate > 0 THEN p.price_rub_minor::float8 / 100.0 / eur_fx.rate::float8
                         ELSE NULL END) OVER w AS previous_price_eur,
                    p.rub_rate_on,
                    usd_fx.rate::float8 AS usd_rub_rate, usd_fx.rate_on AS usd_rate_on,
                    eur_fx.rate::float8 AS eur_rub_rate, eur_fx.rate_on AS eur_rate_on,
                    p.currency::text AS currency,
                    p.price_minor::float8 / 100.0 AS original_price,
                    p.billing_period, p.vat_included, p.price_kind,
                    (t.attributes #>> '{normalized_details,traffic_gib}')::bigint AS traffic_gib,
                    (t.attributes #>> '{normalized_details,traffic_unlimited}')::boolean AS traffic_unlimited,
                    t.attributes->'normalized_details' AS normalized_details,
                    p.raw_excerpt AS page_details, p.source_url, p.parse_status,
                    count(*) OVER (PARTITION BY p.provider_tariff_id) AS history_points,
                    row_number() OVER (PARTITION BY p.provider_tariff_id
                                      ORDER BY p.observed_at DESC, p.id DESC) AS latest
             FROM market_intelligence.tariff_prices_rub p
             JOIN market_intelligence.provider_tariffs t ON t.id = p.provider_tariff_id
             JOIN market_intelligence.providers ON providers.id = t.provider_id
             LEFT JOIN market_intelligence.exchange_rates usd_fx
               ON usd_fx.base = 'USD' AND usd_fx.quote = 'RUB'
              AND usd_fx.requested_on = (p.observed_at AT TIME ZONE 'UTC')::date
             LEFT JOIN market_intelligence.exchange_rates eur_fx
               ON eur_fx.base = 'EUR' AND eur_fx.quote = 'RUB'
              AND eur_fx.requested_on = (p.observed_at AT TIME ZONE 'UTC')::date
             WHERE p.parse_status = 'complete'
               AND p.billing_period = 'month'
               AND p.price_kind <> 'unavailable_or_waitlist'
               AND p.price_rub_minor IS NOT NULL
               AND t.cpu_cores IS NOT NULL AND t.ram_mib IS NOT NULL
               AND t.disk_gib IS NOT NULL
             WINDOW w AS (PARTITION BY p.provider_tariff_id ORDER BY p.observed_at, p.id)
           ), latest AS (
             SELECT * FROM observed WHERE latest = 1
           ), provider_benchmarks AS (
             SELECT provider_code, cpu_cores, ram_gib, disk_gib, lower(disk_type) AS disk_key,
                    lower(region) AS region_key, vat_included, traffic_gib, traffic_unlimited,
                    percentile_cont(0.5) WITHIN GROUP (ORDER BY price_rub) AS provider_median_rub,
                    percentile_cont(0.5) WITHIN GROUP (ORDER BY price_usd) AS provider_median_usd,
                    percentile_cont(0.5) WITHIN GROUP (ORDER BY price_eur) AS provider_median_eur
             FROM latest
             WHERE provider_code <> 'aeza' AND price_rub IS NOT NULL
               AND disk_type IS NOT NULL AND region IS NOT NULL
               AND vat_included IS NOT NULL
               AND (traffic_gib IS NOT NULL OR traffic_unlimited IS TRUE)
               AND price_kind IN ('regular', 'listed_monthly', 'official_catalog')
             GROUP BY provider_code, cpu_cores, ram_gib, disk_gib, lower(disk_type),
                      lower(region), vat_included, traffic_gib, traffic_unlimited
           ), benchmarks AS (
             SELECT cpu_cores, ram_gib, disk_gib, disk_key, region_key,
                    vat_included, traffic_gib, traffic_unlimited,
                    percentile_cont(0.5) WITHIN GROUP (ORDER BY provider_median_rub) AS market_median_rub,
                    CASE WHEN count(provider_median_usd) >= 2
                         THEN percentile_cont(0.5) WITHIN GROUP (ORDER BY provider_median_usd) END AS market_median_usd,
                    CASE WHEN count(provider_median_eur) >= 2
                         THEN percentile_cont(0.5) WITHIN GROUP (ORDER BY provider_median_eur) END AS market_median_eur,
                    count(*)::bigint AS providers_at_median
             FROM provider_benchmarks
             GROUP BY cpu_cores, ram_gib, disk_gib, disk_key, region_key,
                      vat_included, traffic_gib, traffic_unlimited
             HAVING count(*) >= 2
           )
           SELECT latest.provider_code, latest.provider, latest.plan_key, latest.plan_name,
                  latest.cpu_cores, latest.ram_gib,
                  latest.disk_gib, latest.disk_type, latest.region, latest.source_kind,
                  latest.observed_on, latest.effective_at, latest.retrieved_at,
                  latest.price_rub,
                  latest.previous_price_rub,
                  latest.price_usd, latest.previous_price_usd,
                  latest.price_eur, latest.previous_price_eur,
                  latest.rub_rate_on, latest.usd_rub_rate, latest.usd_rate_on,
                  latest.eur_rub_rate, latest.eur_rate_on,
                  CASE WHEN latest.previous_price_rub > 0
                       THEN (latest.price_rub / latest.previous_price_rub - 1.0) * 100.0
                       ELSE NULL END AS change_percent,
                  CASE WHEN latest.previous_price_usd > 0
                       THEN (latest.price_usd / latest.previous_price_usd - 1.0) * 100.0
                       ELSE NULL END AS change_percent_usd,
                  CASE WHEN latest.previous_price_eur > 0
                       THEN (latest.price_eur / latest.previous_price_eur - 1.0) * 100.0
                       ELSE NULL END AS change_percent_eur,
                  latest.history_points, benchmarks.market_median_rub,
                  benchmarks.market_median_usd, benchmarks.market_median_eur,
                  coalesce(benchmarks.providers_at_median, 0)::bigint AS providers_at_median,
                  latest.currency, latest.original_price,
                  latest.billing_period, latest.vat_included, latest.traffic_gib,
                  latest.traffic_unlimited, latest.price_kind, latest.normalized_details,
                  latest.page_details, latest.source_url, latest.parse_status
           FROM latest LEFT JOIN benchmarks
             ON benchmarks.cpu_cores = latest.cpu_cores
            AND benchmarks.ram_gib = latest.ram_gib
            AND benchmarks.disk_gib = latest.disk_gib
            AND benchmarks.disk_key = lower(latest.disk_type)
            AND benchmarks.region_key = lower(latest.region)
            AND benchmarks.vat_included = latest.vat_included
            AND benchmarks.traffic_gib IS NOT DISTINCT FROM latest.traffic_gib
            AND benchmarks.traffic_unlimited IS NOT DISTINCT FROM latest.traffic_unlimited
           ORDER BY latest.provider, latest.plan_name LIMIT 2000"#,
    )
    .fetch_all(&pool)
    .await
    .map_err(|error| {
        eprintln!("monitored prices query failed: {error}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    Ok(Json(rows))
}

async fn price_history(
    State(pool): State<PgPool>,
) -> Result<Json<Vec<TariffHistoryPoint>>, StatusCode> {
    let rows = sqlx::query_as::<_, TariffHistoryPoint>(
        r#"SELECT DISTINCT ON (p.provider_tariff_id, (p.observed_at AT TIME ZONE 'UTC')::date)
                  providers.code AS provider_code, t.plan_key,
                  t.cpu_cores::float8 AS cpu_cores,
                  t.ram_mib::float8 / 1024.0 AS ram_gib,
                  t.disk_gib::float8 AS disk_gib, t.disk_type, t.region,
                  (p.observed_at AT TIME ZONE 'UTC')::date AS observed_on,
                  p.price_rub_minor::float8 / 100.0 AS price_rub,
                  CASE WHEN p.currency = 'USD' THEN p.price_minor::float8 / 100.0
                       WHEN usd_fx.rate > 0 THEN p.price_rub_minor::float8 / 100.0 / usd_fx.rate::float8
                       ELSE NULL END AS price_usd,
                  CASE WHEN p.currency = 'EUR' THEN p.price_minor::float8 / 100.0
                       WHEN eur_fx.rate > 0 THEN p.price_rub_minor::float8 / 100.0 / eur_fx.rate::float8
                       ELSE NULL END AS price_eur,
                  usd_fx.rate::float8 AS usd_rub_rate, usd_fx.rate_on AS usd_rate_on,
                  eur_fx.rate::float8 AS eur_rub_rate, eur_fx.rate_on AS eur_rate_on
           FROM market_intelligence.tariff_prices_rub p
           JOIN market_intelligence.provider_tariffs t ON t.id = p.provider_tariff_id
           JOIN market_intelligence.providers ON providers.id = t.provider_id
           LEFT JOIN market_intelligence.exchange_rates usd_fx
             ON usd_fx.base = 'USD' AND usd_fx.quote = 'RUB'
            AND usd_fx.requested_on = (p.observed_at AT TIME ZONE 'UTC')::date
           LEFT JOIN market_intelligence.exchange_rates eur_fx
             ON eur_fx.base = 'EUR' AND eur_fx.quote = 'RUB'
            AND eur_fx.requested_on = (p.observed_at AT TIME ZONE 'UTC')::date
           WHERE p.parse_status = 'complete' AND p.billing_period = 'month'
             AND p.price_kind <> 'unavailable_or_waitlist'
             AND p.price_rub_minor IS NOT NULL
           ORDER BY p.provider_tariff_id, (p.observed_at AT TIME ZONE 'UTC')::date, p.observed_at DESC, p.id DESC
           LIMIT 50000"#,
    )
    .fetch_all(&pool)
    .await
    .map_err(|error| {
        eprintln!("price history query failed: {error}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    Ok(Json(rows))
}

/// Monthly strict market benchmark. Prices are first reduced to one latest
/// observation per tariff per month, then to one median per provider, so a
/// provider with many plans cannot outweigh other providers.
async fn price_benchmark_history(
    State(pool): State<PgPool>,
) -> Result<Json<Vec<BenchmarkHistoryPoint>>, StatusCode> {
    let rows = sqlx::query_as::<_, BenchmarkHistoryPoint>(
        r#"WITH monthly AS (
             SELECT DISTINCT ON (
                        price.provider_tariff_id,
                        date_trunc('month', price.observed_at AT TIME ZONE 'UTC')::date
                    )
                    date_trunc('month', price.observed_at AT TIME ZONE 'UTC')::date AS month_start,
                    provider.code AS provider_code, tariff.plan_key, tariff.plan_name,
                    tariff.cpu_cores::float8 AS cpu_cores,
                    tariff.ram_mib::float8 / 1024.0 AS ram_gib,
                    tariff.disk_gib::float8 AS disk_gib,
                    lower(btrim(tariff.disk_type)) AS disk_key,
                    btrim(tariff.disk_type) AS disk_type,
                    lower(btrim(tariff.region)) AS region_key,
                    btrim(tariff.region) AS region,
                    price.vat_included,
                    (tariff.attributes #>> '{normalized_details,traffic_gib}')::bigint AS traffic_gib,
                    (tariff.attributes #>> '{normalized_details,traffic_unlimited}')::boolean AS traffic_unlimited,
                    price.price_rub_minor::float8 / 100.0 AS price_rub,
                    CASE WHEN price.currency = 'USD' THEN price.price_minor::float8 / 100.0
                         WHEN usd_fx.rate > 0 THEN price.price_rub_minor::float8 / 100.0 / usd_fx.rate::float8
                         ELSE NULL END AS price_usd,
                    CASE WHEN price.currency = 'EUR' THEN price.price_minor::float8 / 100.0
                         WHEN eur_fx.rate > 0 THEN price.price_rub_minor::float8 / 100.0 / eur_fx.rate::float8
                         ELSE NULL END AS price_eur
             FROM market_intelligence.tariff_prices_rub price
             JOIN market_intelligence.provider_tariffs tariff
               ON tariff.id = price.provider_tariff_id
             JOIN market_intelligence.providers provider ON provider.id = tariff.provider_id
             LEFT JOIN market_intelligence.exchange_rates usd_fx
               ON usd_fx.base = 'USD' AND usd_fx.quote = 'RUB'
              AND usd_fx.requested_on = (price.observed_at AT TIME ZONE 'UTC')::date
             LEFT JOIN market_intelligence.exchange_rates eur_fx
               ON eur_fx.base = 'EUR' AND eur_fx.quote = 'RUB'
              AND eur_fx.requested_on = (price.observed_at AT TIME ZONE 'UTC')::date
             WHERE price.parse_status = 'complete'
               AND price.billing_period = 'month'
               AND price.price_kind IN ('regular', 'listed_monthly', 'official_catalog')
               AND price.price_rub_minor IS NOT NULL
               AND tariff.cpu_cores IS NOT NULL AND tariff.ram_mib IS NOT NULL
               AND tariff.disk_gib IS NOT NULL AND tariff.disk_type IS NOT NULL
               AND nullif(btrim(tariff.region), '') IS NOT NULL
               AND price.vat_included IS NOT NULL
               AND (tariff.attributes #>> '{normalized_details,traffic_gib}' IS NOT NULL
                    OR (tariff.attributes #>> '{normalized_details,traffic_unlimited}')::boolean IS TRUE)
             ORDER BY price.provider_tariff_id,
                      date_trunc('month', price.observed_at AT TIME ZONE 'UTC')::date,
                      price.observed_at DESC, price.id DESC
           ), provider_medians AS (
             SELECT month_start, provider_code, cpu_cores, ram_gib, disk_gib,
                    disk_key, region_key, vat_included, traffic_gib, traffic_unlimited,
                    percentile_cont(0.5) WITHIN GROUP (ORDER BY price_rub) AS provider_median_rub,
                    percentile_cont(0.5) WITHIN GROUP (ORDER BY price_usd) AS provider_median_usd,
                    percentile_cont(0.5) WITHIN GROUP (ORDER BY price_eur) AS provider_median_eur
             FROM monthly
             WHERE provider_code <> 'aeza'
             GROUP BY month_start, provider_code, cpu_cores, ram_gib, disk_gib,
                      disk_key, region_key, vat_included, traffic_gib, traffic_unlimited
           ), market AS (
             SELECT month_start, cpu_cores, ram_gib, disk_gib, disk_key, region_key,
                    vat_included, traffic_gib, traffic_unlimited,
                    percentile_cont(0.5) WITHIN GROUP (ORDER BY provider_median_rub) AS median_rub,
                    CASE WHEN count(provider_median_usd) >= 2
                         THEN percentile_cont(0.5) WITHIN GROUP (ORDER BY provider_median_usd) END AS median_usd,
                    CASE WHEN count(provider_median_eur) >= 2
                         THEN percentile_cont(0.5) WITHIN GROUP (ORDER BY provider_median_eur) END AS median_eur,
                    min(provider_median_rub) AS min_provider_median_rub,
                    max(provider_median_rub) AS max_provider_median_rub,
                    min(provider_median_usd) AS min_provider_median_usd,
                    max(provider_median_usd) AS max_provider_median_usd,
                    min(provider_median_eur) AS min_provider_median_eur,
                    max(provider_median_eur) AS max_provider_median_eur,
                    count(DISTINCT provider_code)::bigint AS provider_count
             FROM provider_medians
             GROUP BY month_start, cpu_cores, ram_gib, disk_gib, disk_key, region_key,
                      vat_included, traffic_gib, traffic_unlimited
             HAVING count(DISTINCT provider_code) >= 2
           )
           SELECT own.month_start, own.plan_key, own.plan_name,
                  own.cpu_cores, own.ram_gib, own.disk_gib, own.disk_type, own.region,
                  own.price_rub, own.price_usd, own.price_eur,
                  market.median_rub AS market_median_rub,
                  market.median_usd AS market_median_usd,
                  market.median_eur AS market_median_eur,
                  market.min_provider_median_rub AS provider_median_min_rub,
                  market.max_provider_median_rub AS provider_median_max_rub,
                  market.min_provider_median_usd AS provider_median_min_usd,
                  market.max_provider_median_usd AS provider_median_max_usd,
                  market.min_provider_median_eur AS provider_median_min_eur,
                  market.max_provider_median_eur AS provider_median_max_eur,
                  market.provider_count AS providers_at_median,
                  (own.price_rub / nullif(market.median_rub, 0) - 1.0) * 100.0 AS gap_percent,
                  (own.price_usd / nullif(market.median_usd, 0) - 1.0) * 100.0 AS gap_percent_usd,
                  (own.price_eur / nullif(market.median_eur, 0) - 1.0) * 100.0 AS gap_percent_eur
           FROM monthly own
           JOIN market USING (month_start, cpu_cores, ram_gib, disk_gib,
                              disk_key, region_key, vat_included,
                              traffic_gib, traffic_unlimited)
           WHERE own.provider_code = 'aeza'
           ORDER BY own.plan_key, own.month_start DESC
           LIMIT 10000"#,
    )
    .fetch_all(&pool)
    .await
    .map_err(|error| {
        eprintln!("price benchmark history query failed: {error}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    Ok(Json(rows))
}

/// Current service state and transaction evidence by linked Aéza product.
/// Pending transactions are not classified as failed renewals.
async fn tariff_client_cohorts(
    State(pool): State<PgPool>,
) -> Result<Json<Vec<TariffClientCohort>>, StatusCode> {
    let rows = sqlx::query_as::<_, TariffClientCohort>(
        r#"WITH transaction_counts AS (
             SELECT "serviceId" AS service_id,
                    count(*) FILTER (WHERE type = 'prolong' AND status = 'performed') AS performed,
                    count(*) FILTER (WHERE type = 'prolong' AND status = 'not_performed') AS pending
             FROM public.transactions GROUP BY "serviceId"
           )
           SELECT product.plan_key,
                  count(*)::bigint AS linked_services,
                  count(DISTINCT service."ownerId")::bigint AS accounts,
                  count(*) FILTER (WHERE service.status = 'prolong_wait')::bigint AS awaiting_renewal,
                  count(*) FILTER (WHERE service.status IN ('deleted', 'blocked', 'suspended'))::bigint AS inactive_services,
                  coalesce(sum(tx.performed), 0)::bigint AS performed_renewals,
                  coalesce(sum(tx.pending), 0)::bigint AS pending_renewal_transactions
           FROM public.services service
           JOIN market_intelligence.tariff_products product ON product.id = service.tariff_product_id
           JOIN market_intelligence.providers provider ON provider.id = product.provider_id
           LEFT JOIN transaction_counts tx ON tx.service_id = service.id
           WHERE provider.code = 'aeza' AND service."ownerId" >= 0
           GROUP BY product.id, product.plan_key ORDER BY product.plan_key"#,
    )
    .fetch_all(&pool)
    .await
    .map_err(|error| {
        eprintln!("tariff client cohorts query failed: {error}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    Ok(Json(rows))
}

async fn price_behavior(
    State(pool): State<PgPool>,
) -> Result<Json<Vec<PriceBehaviorRow>>, StatusCode> {
    let rows = sqlx::query_as::<_, PriceBehaviorRow>(include_str!("../../sql/price_behavior.sql"))
        .fetch_all(&pool)
        .await
        .map_err(|error| {
            eprintln!("price behavior query failed: {error}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
    Ok(Json(rows))
}

async fn client_price_impact(
    State(pool): State<PgPool>,
) -> Result<Json<Vec<InternalPriceImpactRow>>, StatusCode> {
    let rows = sqlx::query_as::<_, InternalPriceImpactRow>(include_str!(
        "../../sql/internal_price_impact.sql"
    ))
    .fetch_all(&pool)
    .await
    .map_err(|error| {
        eprintln!("internal price impact query failed: {error}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    Ok(Json(rows.into_iter().filter(|row| {
        row.account_id.parse::<i64>().is_ok_and(|id| id > 0)
            && !row.account_email.as_deref().unwrap_or("").ends_with("@example.test")
    }).collect()))
}

/// Explicit scenario overlay linked to fixture IDs. It never enters live scores.
async fn client_price_impact_demo(
    State(pool): State<PgPool>,
) -> Result<Json<Vec<serde_json::Value>>, StatusCode> {
    let rows: Vec<serde_json::Value> = sqlx::query_scalar(
        "SELECT event FROM public.price_demo_events ORDER BY changed_at DESC, account_id",
    )
    .fetch_all(&pool)
    .await
    .map_err(|error| {
        eprintln!("price scenario query failed: {error}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    Ok(Json(rows))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v1_contract_preserves_source_service_statuses() {
        for status in ["active", "suspended", "prolong_wait", "deleted", "blocked"] {
            let dto = V1Service {
                id: 10,
                name: "vps-10".into(),
                owner_id: 7,
                payment_term: "month".into(),
                auto_prolong: true,
                created_at: Utc::now(),
                expires_at: Utc::now(),
                status: status.into(),
                price: 990,
            };
            let json = serde_json::to_value(dto).unwrap();
            assert_eq!(json["status"], status);
            assert_eq!(json["ownerId"], 7);
            assert_eq!(json["paymentTerm"], "month");
            assert_eq!(json["autoProlong"], true);
            assert!(json.get("accountId").is_none());
        }
    }

    #[test]
    fn v1_contract_preserves_transaction_status_and_type() {
        for (status, operation_type) in [("performed", "prolong"), ("not_performed", "order")] {
            let dto = V1Transaction {
                id: 11,
                amount: 500,
                status: status.into(),
                performed_at: None,
                created_at: Utc::now(),
                operation_type: operation_type.into(),
                service_id: 10,
            };
            let json = serde_json::to_value(dto).unwrap();
            assert_eq!(json["status"], status);
            assert_eq!(json["type"], operation_type);
            assert_eq!(json["serviceId"], 10);
            assert!(json.get("kind").is_none());
            assert!(json.get("billingType").is_none());
        }
    }

    #[test]
    fn recurring_revenue_uses_billable_current_statuses() {
        for (status, expected) in [
            ("active", 250),
            ("suspended", 250),
            ("prolong_wait", 0),
            ("deleted", 0),
            ("blocked", 0),
        ] {
            let service = ClientService {
                id: "1".into(),
                account_id: "1".into(),
                tariff_product_id: None,
                tariff_plan_key: None,
                name: "service".into(),
                region: None,
                price_cents: 250,
                renewal_price_cents: 250,
                payment_term_seconds: 2592000.0,
                renews_at: Utc::now(),
                auto_renew: true,
                status,
            };
            assert_eq!(recurring_service_revenue(&service), expected, "{status}");
        }
    }

    #[test]
    fn expected_loss_is_score_weighted_clamped_and_saturating() {
        assert_eq!(modeled_expected_loss(10_000, 0), 0);
        assert_eq!(modeled_expected_loss(10_000, 100), 10_000);
        assert_eq!(modeled_expected_loss(101, 50), 51);
        assert_eq!(modeled_expected_loss(10_000, -10), 0);
        assert_eq!(modeled_expected_loss(10_000, 150), 10_000);
        assert_eq!(modeled_expected_loss(-10_000, 100), 0);
        assert_eq!(modeled_expected_loss(i64::MAX, 100), i64::MAX);
    }

    #[test]
    fn summary_excludes_negative_id_fixtures_and_separates_loss_from_payment_deficit() {
        let now = Utc::now();
        let account = |id: &str, email: &str| ClientAccount {
            id: id.into(),
            name: email.into(),
            email: email.into(),
            balance_cents: 0,
            total_replenished_cents: 0,
        };
        let service = |id: &str, account_id: &str, price_cents: i64, status| ClientService {
            id: id.into(),
            account_id: account_id.into(),
            tariff_product_id: None,
            tariff_plan_key: None,
            name: "test-service".into(),
            region: None,
            price_cents,
            renewal_price_cents: price_cents,
            payment_term_seconds: 2_592_000.0,
            renews_at: now,
            auto_renew: true,
            status,
        };
        let risk = |id: &str, score, level, assessment_status, upcoming, balance| SnapshotRisk {
            account_id: id.into(),
            score,
            level,
            factors: vec![],
            upcoming_cents: upcoming,
            monthly_cents: 0,
            topups_cents: 0,
            spendable_balance_cents: balance,
            payment_coverage: None,
            assessment_status,
            available_factors: 0,
            settings_version: 1,
        };
        let state = LiveState {
            settings: SnapshotSettings::default(),
            settings_metadata: SettingsMetadata::default(),
            dataset: ClientDataset {
                accounts: vec![
                    account("1", "high@real.test"),
                    account("2", "low@real.test"),
                    account("3", "limited@real.test"),
                    account("-1", "fixture@example.test"),
                    account("4", "model@example.test"),
                ],
                services: vec![
                    service("1", "1", 1_000, "active"),
                    service("2", "1", 500, "suspended"),
                    service("3", "1", 900, "blocked"),
                    service("4", "1", 1_200, "cancelled"),
                    service("5", "2", 900, "active"),
                    service("6", "3", 700, "active"),
                    service("7", "-1", 50, "active"),
                    service("8", "4", 50, "active"),
                ],
                transactions: vec![],
                as_of: now,
                snapshots: vec![],
            },
            risks: [
                (
                    "1".into(),
                    risk("1", 100, "high", "sufficient_data", 2_000, 1_000),
                ),
                ("2".into(), risk("2", 0, "low", "sufficient_data", 0, 0)),
                ("3".into(), risk("3", 80, "limited", "limited_data", 500, 0)),
                (
                    "-1".into(),
                    risk("-1", 100, "high", "sufficient_data", 0, 0),
                ),
                ("4".into(), risk("4", 100, "high", "sufficient_data", 0, 0)),
            ]
            .into(),
            log_id: None,
            last_calculated_at: now,
        };

        // -1 is a synthetic fixture; 4 is a real customer despite its
        // @example.test address, like every customer of the given dataset.
        let result = summary(&state);
        assert_eq!(result["customersTotal"], 4);
        assert_eq!(result["customersByRisk"]["high"], 2);
        assert_eq!(result["customersByRisk"]["low"], 1);
        assert_eq!(result["customersByRisk"]["limited"], 1);
        assert_eq!(result["revenueAtRiskEurCents"], 1_550);
        assert_eq!(result["paymentDeficitEurCents"], 1_500);
        assert_eq!(result["expectedLossEurCents"], 1_550);
        assert_eq!(result["expectedLossAssessedMonthlyRevenueEurCents"], 2_450);
        assert_eq!(result["expectedLossUnassessedMonthlyRevenueEurCents"], 700);
        assert_eq!(result["expectedLossUnassessedCustomers"], 1);
    }

    #[test]
    fn customer_query_accepts_documented_sort_and_filter_values_and_rejects_invalid_input() {
        let query =
            |sort: Option<&str>, direction: Option<&str>, risk: Option<&str>, page, limit| {
                CustomerQuery {
                    search: None,
                    risk: risk.map(str::to_owned),
                    sort: sort.map(str::to_owned),
                    direction: direction.map(str::to_owned),
                    page,
                    limit,
                }
            };
        for (field, sql) in [
            ("id", "id"),
            ("email", "lower(email)"),
            ("balance", "balance"),
            ("totalReplenished", "\"totalReplenished\""),
            ("score", "id"),
        ] {
            assert_eq!(
                validate_customer_query(&query(
                    Some(field),
                    Some("asc"),
                    Some("all"),
                    Some(1),
                    Some(500)
                )),
                Ok(sql)
            );
        }
        assert!(
            validate_customer_query(&query(
                Some("id"),
                Some("desc"),
                Some("high"),
                Some(2),
                Some(25)
            ))
            .is_ok()
        );
        for invalid in [
            query(
                Some("email); DROP TABLE accounts"),
                Some("asc"),
                None,
                None,
                None,
            ),
            query(Some("id"), Some("sideways"), None, None, None),
            query(Some("id"), Some("asc"), Some("unknown"), None, None),
            query(Some("id"), Some("asc"), None, Some(0), None),
            query(Some("id"), Some("asc"), None, None, Some(501)),
        ] {
            assert_eq!(
                validate_customer_query(&invalid),
                Err(StatusCode::BAD_REQUEST)
            );
        }
    }

    #[test]
    fn descending_score_sort_keeps_numeric_id_tie_breaking_stable() {
        for direction in ["asc", "desc"] {
            assert_eq!(
                compare_customer_scores(50, 2, 50, 10, direction),
                std::cmp::Ordering::Less
            );
            assert_eq!(
                compare_customer_scores(50, 10, 50, 2, direction),
                std::cmp::Ordering::Greater
            );
        }
        assert_eq!(
            compare_customer_scores(90, 10, 20, 2, "asc"),
            std::cmp::Ordering::Greater
        );
        assert_eq!(
            compare_customer_scores(90, 10, 20, 2, "desc"),
            std::cmp::Ordering::Less
        );
    }

    #[test]
    fn price_factor_requires_real_matched_and_repeated_evidence() {
        let now = Utc::now();
        let mut rows = Vec::new();
        for period in 0..2 {
            for band in 0..3 {
                for index in 0..100 {
                    let account = 1 + period * 300 + band * 100 + index;
                    let renewed = index < 80 - band * 15;
                    rows.push(InternalPriceImpactRow {
                        event_id: account.to_string(),
                        service_id: account.to_string(),
                        account_id: account.to_string(),
                        account_email: Some(format!("client{account}@real.test")),
                        plan_key: None,
                        plan_name: "TEST".into(),
                        changed_at: now
                            - chrono::Duration::days(if period == 0 { 270 } else { 75 }),
                        previous_price_cents: 1000,
                        current_price_cents: [1050, 1150, 1300][band as usize],
                        next_renewal_at: now
                            - chrono::Duration::days(if period == 0 { 255 } else { 60 }),
                        payment_term: "month".into(),
                        complete_baseline: true,
                        complete_pretrend: true,
                        complete_followup: true,
                        renewal_due_in_window: true,
                        current_status: Some("active".into()),
                        current_auto_prolong: Some(true),
                        renewals_earlier: 1,
                        renewals_before: 1,
                        renewals_after: i64::from(renewed),
                        prolong_wait_after: !renewed,
                        auto_renew_disabled_after: false,
                        inactive_after: false,
                        baseline_services_count: Some(1),
                        followup_services_count: Some(1),
                        baseline_monthly_spend: Some(if index % 2 == 0 { 800.0 } else { 1200.0 }),
                        followup_monthly_spend: Some(1000.0),
                        account_spend_earlier_cents: 1000,
                        account_spend_before_cents: 1000,
                        account_spend_after_cents: 1000,
                        control_account_id: Some(format!("control-{account}")),
                        control_services_before: Some(1),
                        control_services_after: Some(1),
                        control_spend_before_cents: Some(1000),
                        control_spend_after_cents: Some(1000),
                        control_renewed_after: Some(index < 80),
                        control_renewals_earlier: Some(1),
                        control_renewals_before: Some(1),
                        control_spend_earlier_cents: Some(1000),
                    });
                }
            }
        }
        assert!(!validated_price_severity(&rows, now).is_empty());
        rows[0].renewals_before = 0;
        assert!(validated_price_severity(&rows, now).is_empty());
        rows[0].renewals_before = 1;
        rows[0].account_email = Some("fixture@example.test".into());
        assert!(validated_price_severity(&rows, now).is_empty());
        rows[0].account_email = Some("client1@real.test".into());
        for row in rows.iter_mut().filter(|row| {
            row.changed_at >= now - chrono::Duration::days(183) && row.current_price_cents == 1300
        }) {
            row.renewals_after = 1;
            row.prolong_wait_after = false;
        }
        assert!(validated_price_severity(&rows, now).is_empty());
    }

    #[test]
    fn snapshot_risk_keeps_price_factor_unavailable_without_evidence() {
        let settings: SnapshotSettings = serde_json::from_str(r#"{"weights":{"paymentCoverage":1,"noAutoProlong":1,"prolongWait":1,"failedProlong":1,"expiringRevenue":1},"mediumThreshold":40,"highThreshold":70,"renewalWindowDays":7}"#).unwrap();
        let client = ClientFeatures {
            account_id: 1,
            email: "fixture@example.test".into(),
            services_count: 1,
            monthly_spend: 1000.0,
            short_term_share: Some(0.0),
            no_auto_prolong_share: Some(1.0),
            prolong_wait_share: Some(1.0),
            failed_prolong_share: None,
            expiring_revenue_share: Some(1.0),
            upcoming_payments: 1000,
            spendable_balance: 0,
            payment_coverage: Some(0.0),
        };
        let risk = calculate_snapshot_risk(&client, 500, &settings, None);
        assert_eq!(risk.score, 100);
        assert_eq!(risk.level, "high");
        assert_eq!(risk.upcoming_cents, 1000);
        assert_eq!(risk.factors.len(), 6);
        assert!(
            !risk
                .factors
                .iter()
                .find(|factor| factor.key == "priceRisk")
                .unwrap()
                .available
        );
        let json = serde_json::to_value(&risk).unwrap();
        assert_eq!(json["upcomingCents"], 1000);
        assert_eq!(json["accountId"], "1");
        assert_eq!(json["settingsVersion"], 0);
    }

    #[test]
    fn every_model_weight_changes_its_normalized_factor_contribution() {
        let client = ClientFeatures {
            account_id: 1,
            email: "weights@example.test".into(),
            services_count: 4,
            monthly_spend: 1_000.0,
            short_term_share: Some(0.25),
            no_auto_prolong_share: Some(0.60),
            prolong_wait_share: Some(0.30),
            failed_prolong_share: Some(0.50),
            expiring_revenue_share: Some(0.40),
            upcoming_payments: 1_000,
            spendable_balance: 200,
            payment_coverage: Some(0.20),
        };
        let baseline = SnapshotSettings {
            weights: SnapshotWeights {
                payment_coverage: 10.0,
                no_auto_prolong: 10.0,
                prolong_wait: 10.0,
                failed_prolong: 10.0,
                expiring_revenue: 10.0,
                price_risk: 10.0,
            },
            medium_threshold: 35,
            high_threshold: 65,
            renewal_window_days: 14,
            historical_min_days: 30,
        };
        let expected_factor_keys = [
            ("paymentCoverage", "payment_coverage"),
            ("noAutoProlongShare", "no_auto_prolong"),
            ("prolongWaitShare", "prolong_wait"),
            ("failedProlongShare", "failed_prolong"),
            ("expiringRevenueShare", "expiring_revenue"),
            ("priceRisk", "price_risk"),
        ];

        for (factor_key, weight_field) in expected_factor_keys {
            let baseline_risk = calculate_snapshot_risk(&client, 0, &baseline, Some(0.80));
            let baseline_points = baseline_risk
                .factors
                .iter()
                .find(|factor| factor.key == factor_key)
                .unwrap()
                .points;
            let mut updated = SnapshotSettings {
                weights: SnapshotWeights {
                    payment_coverage: 10.0,
                    no_auto_prolong: 10.0,
                    prolong_wait: 10.0,
                    failed_prolong: 10.0,
                    expiring_revenue: 10.0,
                    price_risk: 10.0,
                },
                medium_threshold: 35,
                high_threshold: 65,
                renewal_window_days: 14,
                historical_min_days: 30,
            };
            match weight_field {
                "payment_coverage" => updated.weights.payment_coverage = 60.0,
                "no_auto_prolong" => updated.weights.no_auto_prolong = 60.0,
                "prolong_wait" => updated.weights.prolong_wait = 60.0,
                "failed_prolong" => updated.weights.failed_prolong = 60.0,
                "expiring_revenue" => updated.weights.expiring_revenue = 60.0,
                "price_risk" => updated.weights.price_risk = 60.0,
                _ => unreachable!(),
            }
            let updated_risk = calculate_snapshot_risk(&client, 0, &updated, Some(0.80));
            let updated_points = updated_risk
                .factors
                .iter()
                .find(|factor| factor.key == factor_key)
                .unwrap()
                .points;
            assert_ne!(updated_points, baseline_points, "{factor_key} weight");
            assert_eq!(
                updated_risk.score,
                updated_risk
                    .factors
                    .iter()
                    .map(|factor| factor.points)
                    .sum::<f64>()
                    .round() as i64,
                "{factor_key} keeps the normalized score equal to the factor sum"
            );
        }
    }

    #[test]
    fn settings_reject_negative_non_finite_and_out_of_range_weights() {
        let mut settings = SnapshotSettings::default();
        assert!(settings.is_valid());

        settings.weights.payment_coverage = -0.01;
        assert!(!settings.is_valid());
        settings.weights.payment_coverage = 35.0;
        settings.weights.price_risk = 100.01;
        assert!(!settings.is_valid());
        settings.weights.price_risk = f64::INFINITY;
        assert!(!settings.is_valid());
        settings.weights.price_risk = 0.0;
        settings.weights.payment_coverage = 0.0;
        settings.weights.no_auto_prolong = 0.0;
        settings.weights.prolong_wait = 0.0;
        settings.weights.failed_prolong = 0.0;
        settings.weights.expiring_revenue = 0.0;
        assert!(!settings.is_valid());
    }

    #[test]
    fn client_json_matches_frontend_field_names() {
        let transaction = ClientTransaction {
            id: "1".into(),
            account_id: "2".into(),
            service_id: "3".into(),
            tariff_product_id: Some("4".into()),
            amount_cents: 500,
            kind: "payment",
            billing_type: "prolong".into(),
            status: "completed",
            created_at: Utc::now(),
        };
        let json = serde_json::to_value(transaction).unwrap();
        assert_eq!(json["accountId"], "2");
        assert_eq!(json["serviceId"], "3");
        assert_eq!(json["amountCents"], 500);
        assert_eq!(json["type"], "payment");
        assert!(json.get("kind").is_none());
    }

    #[test]
    fn snapshot_risk_uses_the_five_measured_features_and_the_price_factor() {
        let settings: SnapshotSettings = serde_json::from_str(r#"{"weights":{"paymentCoverage":1,"noAutoProlong":1,"prolongWait":1,"failedProlong":1,"expiringRevenue":1},"mediumThreshold":40,"highThreshold":70,"renewalWindowDays":7}"#).unwrap();
        let client = ClientFeatures {
            account_id: 1,
            email: "fixture@example.test".into(),
            services_count: 1,
            monthly_spend: 1000.0,
            short_term_share: Some(0.0),
            no_auto_prolong_share: Some(1.0),
            prolong_wait_share: Some(1.0),
            failed_prolong_share: None,
            expiring_revenue_share: Some(1.0),
            upcoming_payments: 1000,
            spendable_balance: 0,
            payment_coverage: Some(0.0),
        };
        let risk = calculate_snapshot_risk(&client, 500, &settings, None);
        assert_eq!(risk.score, 100);
        assert_eq!(risk.level, "high");
        assert_eq!(risk.upcoming_cents, 1000);
        // The five measured features, plus the price factor without evidence.
        assert_eq!(risk.factors.len(), 6);
        assert!(risk.factors.iter().any(|factor| factor.key == "priceRisk" && !factor.available));
        let json = serde_json::to_value(&risk).unwrap();
        assert_eq!(json["upcomingCents"], 1000);
        assert_eq!(json["accountId"], "1");
        assert_eq!(json["settingsVersion"], 0);
    }

    #[test]
    fn new_weights_and_thresholds_change_the_risk_result() {
        let client = ClientFeatures {
            account_id: 1,
            email: "fixture@example.test".into(),
            services_count: 1,
            monthly_spend: 1000.0,
            short_term_share: Some(0.0),
            no_auto_prolong_share: Some(1.0),
            prolong_wait_share: Some(0.0),
            failed_prolong_share: None,
            expiring_revenue_share: Some(0.0),
            upcoming_payments: 1000,
            spendable_balance: 1000,
            payment_coverage: Some(1.0),
        };
        let baseline = SnapshotSettings::default();
        let baseline_risk = calculate_snapshot_risk(&client, 0, &baseline, None);
        assert_eq!(baseline_risk.score, 17);
        assert_eq!(baseline_risk.level, "low");

        let thresholded = SnapshotSettings {
            medium_threshold: 10,
            high_threshold: 20,
            ..SnapshotSettings::default()
        };
        let thresholded_risk = calculate_snapshot_risk(&client, 0, &thresholded, None);
        assert_eq!(thresholded_risk.score, baseline_risk.score);
        assert_eq!(thresholded_risk.level, "medium");

        let weighted = SnapshotSettings {
            weights: SnapshotWeights {
                payment_coverage: 0.0,
                no_auto_prolong: 100.0,
                prolong_wait: 0.0,
                failed_prolong: 0.0,
                expiring_revenue: 0.0,
                price_risk: 0.0,
            },
            medium_threshold: 20,
            high_threshold: 80,
            renewal_window_days: 14,
            historical_min_days: 30,
        };
        let weighted_risk = calculate_snapshot_risk(&client, 0, &weighted, None);
        assert_eq!(weighted_risk.score, 100);
        assert_eq!(weighted_risk.level, "high");
    }

    #[test]
    fn live_messages_use_frontend_field_names() {
        let message: ClientMessage = serde_json::from_str(r#"{"type":"updateSettings","requestId":"7","settings":{"weights":{"paymentCoverage":1,"noAutoProlong":1,"prolongWait":1,"failedProlong":1,"expiringRevenue":1},"mediumThreshold":40,"highThreshold":70,"renewalWindowDays":7,"historicalMinDays":30}}"#).unwrap();
        let ClientMessage::UpdateSettings {
            request_id,
            settings,
        } = message;
        assert_eq!(request_id, "7");
        assert!(settings.is_valid());
        let metadata = SettingsMetadata::default();
        let ack: serde_json::Value = serde_json::from_str(
            encode(&ServerMessage::Ack {
                request_id: "7",
                settings_metadata: &metadata,
            })
            .as_str(),
        )
        .unwrap();
        assert_eq!(
            ack,
            serde_json::json!({
                "type": "ack", "requestId": "7",
                "settingsMetadata": {"version": 0, "updatedAt": null, "updatedBy": null}
            })
        );
        let defaults = serde_json::to_value(SnapshotSettings::default()).unwrap();
        assert_eq!(defaults["weights"]["paymentCoverage"], 35.0);
        assert_eq!(defaults["historicalMinDays"], 30);
        assert!(SnapshotSettings::default().is_valid());
    }

    #[test]
    fn score_changes_lists_only_changed_and_removed_accounts() {
        let client = |account_id, no_auto_prolong_share| ClientFeatures {
            account_id,
            email: format!("{account_id}@example.test"),
            services_count: 1,
            monthly_spend: 1000.0,
            short_term_share: Some(0.0),
            no_auto_prolong_share: Some(no_auto_prolong_share),
            prolong_wait_share: Some(0.0),
            failed_prolong_share: Some(0.0),
            expiring_revenue_share: Some(0.0),
            upcoming_payments: 0,
            spendable_balance: 0,
            payment_coverage: Some(10.0),
        };
        let settings = SnapshotSettings::default();
        let risks = |clients: &[ClientFeatures]| -> Risks {
            clients
                .iter()
                .map(|client| {
                    let risk = calculate_snapshot_risk(client, 0, &settings, None);
                    (risk.account_id.clone(), risk)
                })
                .collect()
        };
        let old = risks(&[client(1, 0.0), client(2, 0.0), client(3, 0.0)]);
        let new = risks(&[client(1, 0.0), client(2, 1.0)]);
        let (changed, removed) = score_changes(&old, &new);
        assert_eq!(
            changed
                .iter()
                .map(|risk| risk.account_id.as_str())
                .collect::<Vec<_>>(),
            ["2"]
        );
        assert_eq!(removed, ["3"]);
        let (changed, removed) = score_changes(&new, &new);
        assert!(changed.is_empty() && removed.is_empty());
    }

    #[test]
    fn empty_client_has_zero_risk_and_no_division_by_zero() {
        let client = ClientFeatures {
            account_id: -1,
            email: "empty@example.test".into(),
            services_count: 0,
            monthly_spend: 0.0,
            short_term_share: None,
            no_auto_prolong_share: None,
            prolong_wait_share: None,
            failed_prolong_share: None,
            expiring_revenue_share: None,
            upcoming_payments: 0,
            spendable_balance: 0,
            payment_coverage: None,
        };
        let settings: SnapshotSettings = serde_json::from_str(r#"{"weights":{"paymentCoverage":35,"noAutoProlong":15,"prolongWait":15,"failedProlong":10,"expiringRevenue":25},"mediumThreshold":35,"highThreshold":65,"renewalWindowDays":14}"#).unwrap();
        let risk = calculate_snapshot_risk(&client, 0, &settings, None);
        assert_eq!(risk.score, 0);
        assert_eq!(risk.level, "insufficient");
        assert_eq!(risk.upcoming_cents, 0);
        assert!(risk.factors.iter().all(|factor| factor.severity == 0.0));
    }
}
