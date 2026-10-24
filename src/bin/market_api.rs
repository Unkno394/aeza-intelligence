use std::net::SocketAddr;

use aeza_intelligence::{
    PgAdapter,
    analysis::{self, Params},
    historical::{HistoricalModel, lcs_similarity},
    model::ClientFeatures,
    sequences::{DiscoveredPattern, discover_patterns},
};
use anyhow::Result;
use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
};
use chrono::{DateTime, NaiveDate, Utc};
use clap::Parser;
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row, postgres::PgPoolOptions};

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
    observed_on: NaiveDate,
    price_rub: Option<f64>,
    previous_price_rub: Option<f64>,
    change_percent: Option<f64>,
    history_points: i64,
    market_median_rub: Option<f64>,
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
    assessment: serde_json::Value,
    features: serde_json::Value,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SequencePeer {
    account_id: i64,
    similarity: f64,
    sequence: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SnapshotSettings {
    weights: SnapshotWeights,
    medium_threshold: i64,
    high_threshold: i64,
    renewal_window_days: i64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SnapshotWeights {
    payment_coverage: f64,
    no_auto_prolong: f64,
    prolong_wait: f64,
    failed_prolong: f64,
    expiring_revenue: f64,
}

#[derive(Serialize)]
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
    payment_coverage: f64,
}

#[derive(Serialize)]
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
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&cli.database_url)
        .await?;
    PgAdapter::from_pool(pool.clone()).migrate().await?;
    let app = Router::new()
        .route("/api/market/search-interest", get(search_interest))
        .route(
            "/api/market/search-interest/comparison",
            get(search_interest_comparison),
        )
        .route("/api/market/geography", get(geography))
        .route("/api/market/prices", get(prices))
        .route("/api/market/prices/monitor", get(monitored_prices))
        .route("/api/market/prices/history", get(price_history))
        .route(
            "/api/market/prices/client-cohorts",
            get(tariff_client_cohorts),
        )
        .route("/api/market/prices/behavior", get(price_behavior))
        .route("/api/market/prices/quality", get(price_quality))
        .route("/api/client/dataset", get(client_dataset))
        .route("/api/client/analysis", get(client_analysis))
        .route("/api/client/financial-ledger/{id}", get(financial_ledger))
        .route("/api/client/data-quality", get(client_data_quality))
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
        .with_state(pool);
    let listener = tokio::net::TcpListener::bind(cli.listen).await?;
    eprintln!("market API listening on http://{}", cli.listen);
    axum::serve(listener, app).await?;
    Ok(())
}

async fn client_dataset(State(pool): State<PgPool>) -> Result<Json<ClientDataset>, StatusCode> {
    let account_rows = sqlx::query(
        "SELECT id::bigint AS id, email, balance, \"totalReplenished\" \
         FROM public.accounts ORDER BY id",
    )
    .fetch_all(&pool)
    .await
    .map_err(db_error)?;
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
    .fetch_all(&pool)
    .await
    .map_err(db_error)?;
    let transaction_rows = sqlx::query(
        "SELECT t.id::bigint AS id, s.\"ownerId\"::bigint AS owner_id, \
         t.\"serviceId\"::bigint AS service_id, t.amount::bigint AS amount, \
         s.tariff_product_id::bigint AS tariff_product_id, \
         t.type::text AS type, t.status::text AS status, t.\"createdAt\" \
         FROM public.transactions t JOIN public.services s ON s.id = t.\"serviceId\" ORDER BY t.id",
    )
    .fetch_all(&pool)
    .await
    .map_err(db_error)?;
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
                    "failed"
                },
                created_at: row.get("createdAt"),
            }
        })
        .collect();
    Ok(Json(ClientDataset {
        accounts,
        services,
        transactions,
        as_of: Utc::now(),
        snapshots: Vec::new(),
    }))
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

async fn client_data_quality(
    State(pool): State<PgPool>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let row = sqlx::query(
        "SELECT \
         (SELECT count(*) FROM public.accounts WHERE id >= 0 \
          AND email NOT LIKE '%@example.test') AS real_accounts, \
         (SELECT count(DISTINCT s.account_id) FROM public.client_snapshots s \
          JOIN public.accounts a ON a.id = s.account_id \
          WHERE a.id >= 0 AND a.email NOT LIKE '%@example.test' \
            AND s.observed_on = (now() AT TIME ZONE 'UTC')::date) AS today_accounts, \
         (SELECT count(*) FROM (SELECT s.account_id FROM public.client_snapshots s \
          JOIN public.accounts a ON a.id = s.account_id \
          WHERE a.id >= 0 AND a.email NOT LIKE '%@example.test' \
          GROUP BY s.account_id \
          HAVING count(*) >= 10 AND max(s.observed_on) - min(s.observed_on) >= 30) ready) AS real_accounts_with_history, \
         (SELECT max(s.observed_on) FROM public.client_snapshots s \
          JOIN public.accounts a ON a.id = s.account_id \
          WHERE a.id >= 0 AND a.email NOT LIKE '%@example.test') AS latest_real_snapshot, \
         (SELECT count(*) FROM public.services s \
          JOIN public.accounts a ON a.id = s.\"ownerId\" \
          WHERE s.tariff_product_id IS NULL AND a.id >= 0 \
            AND a.email NOT LIKE '%@example.test') AS unlinked_services, \
         (SELECT count(*) FROM public.transactions WHERE type = 'prolong' \
          AND status = 'not_performed') AS ambiguous_prolongs, \
         (SELECT count(*) FROM public.client_snapshots s \
          JOIN public.accounts a ON a.id = s.account_id \
          WHERE a.id >= 0 AND a.email NOT LIKE '%@example.test' \
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
        "ambiguousProlongs": row.get::<i64, _>("ambiguous_prolongs"),
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
    let weights = &settings.weights;
    let all = [
        weights.payment_coverage,
        weights.no_auto_prolong,
        weights.prolong_wait,
        weights.failed_prolong,
        weights.expiring_revenue,
    ];
    if all
        .iter()
        .any(|weight| !weight.is_finite() || *weight < 0.0)
        || all.iter().sum::<f64>() <= 0.0
        || !(1..=90).contains(&settings.renewal_window_days)
        || !(1..=100).contains(&settings.medium_threshold)
        || settings.high_threshold <= settings.medium_threshold
        || settings.high_threshold > 100
    {
        return Err(StatusCode::BAD_REQUEST);
    }
    let horizon = format!("{} days", settings.renewal_window_days);
    let clients = PgAdapter::from_pool(pool.clone())
        .client_features(Utc::now(), &horizon)
        .await
        .map_err(|error| {
            eprintln!("snapshot risk feature query failed: {error}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
    let replenished_rows = sqlx::query(
        "SELECT id::bigint AS id, \"totalReplenished\" AS total_replenished FROM public.accounts",
    )
    .fetch_all(&pool)
    .await
    .map_err(db_error)?;
    let replenished: std::collections::BTreeMap<i64, i64> = replenished_rows
        .into_iter()
        .map(|row| (row.get("id"), row.get("total_replenished")))
        .collect();
    Ok(Json(
        clients
            .iter()
            .map(|client| {
                calculate_snapshot_risk(
                    client,
                    replenished
                        .get(&client.account_id)
                        .copied()
                        .unwrap_or_default(),
                    &settings,
                )
            })
            .collect(),
    ))
}

fn calculate_snapshot_risk(
    client: &ClientFeatures,
    topups_cents: i64,
    config: &SnapshotSettings,
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
            "Доля продлений со статусом not_performed.",
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
            Some(client.payment_coverage),
            config.weights.payment_coverage,
            "Доступный баланс / платежи в ближайшем окне; риск возникает ниже 1.",
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
            SnapshotFactor {
                key,
                label,
                severity,
                points: if available && total > 0.0 {
                    severity * weight / total * 100.0
                } else {
                    0.0
                },
                value,
                available,
                explanation: explanation.to_owned(),
            }
        })
        .collect();
    let score = factors
        .iter()
        .map(|factor| factor.points)
        .sum::<f64>()
        .round()
        .clamp(0.0, 100.0) as i64;
    SnapshotRisk {
        account_id: client.account_id.to_string(),
        score,
        level: if score >= config.high_threshold {
            "high"
        } else if score >= config.medium_threshold {
            "medium"
        } else {
            "low"
        },
        factors,
        upcoming_cents: client.upcoming_payments,
        monthly_cents: client.monthly_spend.round() as i64,
        topups_cents,
        spendable_balance_cents: client.spendable_balance,
        payment_coverage: client.payment_coverage,
    }
}

async fn client_history(
    State(pool): State<PgPool>,
    Path(id): Path<i64>,
) -> Result<Json<Vec<HistoryPoint>>, StatusCode> {
    let rows = sqlx::query_as::<_, HistoryPoint>(
        "SELECT observed_on, assessment, features FROM public.client_snapshots \
         WHERE account_id = $1 ORDER BY observed_on DESC LIMIT 365",
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

async fn prices(State(pool): State<PgPool>) -> Result<Json<Vec<Price>>, StatusCode> {
    let rows = sqlx::query_as::<_, Price>(
        "SELECT DISTINCT ON (p.provider_tariff_id) providers.name AS provider, \
         p.plan_name, p.observed_at::date AS observed_on, \
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
                    p.observed_at::date AS observed_on,
                    p.price_rub_minor::float8 / 100.0 AS price_rub,
                    lag(p.price_rub_minor::float8 / 100.0) OVER w AS previous_price_rub,
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
                    percentile_cont(0.5) WITHIN GROUP (ORDER BY price_rub) AS provider_median_rub
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
                    count(*)::bigint AS providers_at_median
             FROM provider_benchmarks
             GROUP BY cpu_cores, ram_gib, disk_gib, disk_key, region_key,
                      vat_included, traffic_gib, traffic_unlimited
             HAVING count(*) >= 2
           )
           SELECT latest.provider_code, latest.provider, latest.plan_key, latest.plan_name,
                  latest.cpu_cores, latest.ram_gib,
                  latest.disk_gib, latest.disk_type, latest.region, latest.observed_on,
                  latest.price_rub,
                  latest.previous_price_rub,
                  CASE WHEN latest.previous_price_rub > 0
                       THEN (latest.price_rub / latest.previous_price_rub - 1.0) * 100.0
                       ELSE NULL END AS change_percent,
                  latest.history_points, benchmarks.market_median_rub,
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
        r#"SELECT DISTINCT ON (p.provider_tariff_id, p.observed_at::date)
                  providers.code AS provider_code, t.plan_key,
                  t.cpu_cores::float8 AS cpu_cores,
                  t.ram_mib::float8 / 1024.0 AS ram_gib,
                  t.disk_gib::float8 AS disk_gib, t.disk_type, t.region,
                  p.observed_at::date AS observed_on,
                  p.price_rub_minor::float8 / 100.0 AS price_rub
           FROM market_intelligence.tariff_prices_rub p
           JOIN market_intelligence.provider_tariffs t ON t.id = p.provider_tariff_id
           JOIN market_intelligence.providers ON providers.id = t.provider_id
           WHERE p.parse_status = 'complete' AND p.billing_period = 'month'
             AND p.price_kind <> 'unavailable_or_waitlist'
             AND p.price_rub_minor IS NOT NULL
           ORDER BY p.provider_tariff_id, p.observed_at::date, p.observed_at DESC, p.id DESC
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
    let rows = sqlx::query_as::<_, PriceBehaviorRow>(include_str!("../sql/price_behavior.sql"))
        .fetch_all(&pool)
        .await
        .map_err(|error| {
            eprintln!("price behavior query failed: {error}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
    Ok(Json(rows))
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn snapshot_risk_uses_the_five_current_risk_features() {
        let settings: SnapshotSettings = serde_json::from_str(r#"{"weights":{"paymentCoverage":1,"noAutoProlong":1,"prolongWait":1,"failedProlong":1,"expiringRevenue":1},"mediumThreshold":40,"highThreshold":70,"renewalWindowDays":7}"#).unwrap();
        let client = ClientFeatures {
            account_id: 1,
            email: "fixture@example.test".into(),
            services_count: 1,
            monthly_spend: 1000.0,
            short_term_share: Some(0.0),
            no_auto_prolong_share: Some(1.0),
            prolong_wait_share: Some(1.0),
            failed_prolong_share: Some(1.0),
            expiring_revenue_share: Some(1.0),
            upcoming_payments: 1000,
            spendable_balance: 0,
            payment_coverage: 0.0,
        };
        let risk = calculate_snapshot_risk(&client, 500, &settings);
        assert_eq!(risk.score, 100);
        assert_eq!(risk.level, "high");
        assert_eq!(risk.upcoming_cents, 1000);
        assert_eq!(risk.factors.len(), 5);
        let json = serde_json::to_value(&risk).unwrap();
        assert_eq!(json["upcomingCents"], 1000);
        assert_eq!(json["accountId"], "1");
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
            payment_coverage: 10.0,
        };
        let settings: SnapshotSettings = serde_json::from_str(r#"{"weights":{"paymentCoverage":35,"noAutoProlong":15,"prolongWait":15,"failedProlong":10,"expiringRevenue":25},"mediumThreshold":35,"highThreshold":65,"renewalWindowDays":14}"#).unwrap();
        let risk = calculate_snapshot_risk(&client, 0, &settings);
        assert_eq!(risk.score, 0);
        assert_eq!(risk.level, "low");
        assert_eq!(risk.upcoming_cents, 0);
        assert!(risk.factors.iter().all(|factor| factor.severity == 0.0));
    }
}
