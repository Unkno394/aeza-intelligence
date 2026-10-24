use std::net::SocketAddr;

use aeza_intelligence::{
    PgAdapter,
    analysis::{self, Params},
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
    amount_cents: i64,
    #[serde(rename = "type")]
    kind: &'static str,
    status: &'static str,
    created_at: DateTime<Utc>,
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
    balance: f64,
    renewal: f64,
    auto_renew: f64,
    status: f64,
    transactions: f64,
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
}

#[derive(Serialize)]
struct SnapshotFactor {
    key: &'static str,
    label: &'static str,
    severity: f64,
    points: f64,
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
        .route("/api/market/geography", get(geography))
        .route("/api/market/prices", get(prices))
        .route("/api/client/dataset", get(client_dataset))
        .route("/api/client/analysis", get(client_analysis))
        .route("/api/client/snapshot", post(client_snapshot_risks))
        .route("/api/client/historical/{id}", get(client_history))
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
        "SELECT id::bigint AS id, \"ownerId\"::bigint AS owner_id, name, \
         round(price::numeric * 2592000 / public.payment_term_seconds(\"paymentTerm\"::text))::bigint AS monthly_price, \
         price AS renewal_price, public.payment_term_seconds(\"paymentTerm\"::text) AS payment_term_seconds, \
         \"expiresAt\", \"autoProlong\", status::text AS status \
         FROM public.services ORDER BY id",
    )
    .fetch_all(&pool)
    .await
    .map_err(db_error)?;
    let transaction_rows = sqlx::query(
        "SELECT t.id::bigint AS id, s.\"ownerId\"::bigint AS owner_id, \
         t.\"serviceId\"::bigint AS service_id, t.amount::bigint AS amount, \
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
                name: row.get("name"),
                region: None,
                price_cents: row.get("monthly_price"),
                renewal_price_cents: row.get("renewal_price"),
                payment_term_seconds: row.get("payment_term_seconds"),
                renews_at: row.get("expiresAt"),
                auto_renew: row.get("autoProlong"),
                status: match status.as_str() {
                    "deleted" => "cancelled",
                    "active" => "active",
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
                amount_cents: row.get("amount"),
                kind: "payment",
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
        weights.balance,
        weights.renewal,
        weights.auto_renew,
        weights.status,
        weights.transactions,
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
    let dataset = client_dataset(State(pool)).await?.0;
    Ok(Json(
        dataset
            .accounts
            .iter()
            .map(|account| calculate_snapshot_risk(account, &dataset, &settings))
            .collect(),
    ))
}

fn calculate_snapshot_risk(
    account: &ClientAccount,
    data: &ClientDataset,
    config: &SnapshotSettings,
) -> SnapshotRisk {
    let services: Vec<_> = data
        .services
        .iter()
        .filter(|service| service.account_id == account.id && service.status != "cancelled")
        .collect();
    let transactions: Vec<_> = data
        .transactions
        .iter()
        .filter(|transaction| transaction.account_id == account.id)
        .collect();
    let window_end = data.as_of + chrono::Duration::days(config.renewal_window_days);
    let upcoming_cents = services
        .iter()
        .filter(|service| service.renews_at <= window_end)
        .map(|service| {
            let term_ms = service.payment_term_seconds * 1_000.0;
            let start = service.renews_at.max(data.as_of);
            let count = if term_ms.is_finite() && term_ms > 0.0 {
                ((window_end - start).num_milliseconds() as f64 / term_ms)
                    .ceil()
                    .max(1.0) as i64
            } else {
                1
            };
            count.saturating_mul(service.renewal_price_cents)
        })
        .sum::<i64>();
    let monthly_cents = services
        .iter()
        .map(|service| service.price_cents)
        .sum::<i64>();
    let recent: Vec<_> = transactions
        .iter()
        .filter(|transaction| transaction.created_at >= data.as_of - chrono::Duration::days(30))
        .collect();
    let failures = recent
        .iter()
        .filter(|transaction| transaction.status == "failed")
        .count();
    let renewal_severity = services
        .iter()
        .map(|service| {
            let days = (service.renews_at - data.as_of).num_milliseconds() as f64 / 86_400_000.0;
            (1.0 - days.max(0.0) / config.renewal_window_days as f64).max(0.0)
        })
        .fold(0.0, f64::max);
    let values = [
        (
            "balance",
            "Покрытие балансом",
            if upcoming_cents > 0 {
                (1.0 - account.balance_cents as f64 / upcoming_cents as f64).max(0.0)
            } else {
                0.0
            },
            config.weights.balance,
            format!(
                "Баланс {} центов; продления в окне — {} центов.",
                account.balance_cents, upcoming_cents
            ),
        ),
        (
            "renewal",
            "Близость продления",
            renewal_severity,
            config.weights.renewal,
            format!(
                "Ближайшее продление в окне {} дней.",
                config.renewal_window_days
            ),
        ),
        (
            "autoRenew",
            "Автопродление отключено",
            if services.is_empty() {
                0.0
            } else {
                services
                    .iter()
                    .filter(|service| !service.auto_renew)
                    .count() as f64
                    / services.len() as f64
            },
            config.weights.auto_renew,
            "Доля услуг с отключённым автопродлением.".to_owned(),
        ),
        (
            "status",
            "Приостановленные услуги",
            if services.is_empty() {
                0.0
            } else {
                services
                    .iter()
                    .filter(|service| service.status == "suspended")
                    .count() as f64
                    / services.len() as f64
            },
            config.weights.status,
            "Доля приостановленных услуг.".to_owned(),
        ),
        (
            "transactions",
            "Неуспешные транзакции",
            if recent.is_empty() {
                0.0
            } else {
                failures as f64 / recent.len() as f64
            },
            config.weights.transactions,
            format!(
                "{} из {} транзакций за 30 дней завершились ошибкой.",
                failures,
                recent.len()
            ),
        ),
    ];
    let total = [
        config.weights.balance,
        config.weights.renewal,
        config.weights.auto_renew,
        config.weights.status,
        config.weights.transactions,
    ]
    .iter()
    .sum::<f64>();
    let factors: Vec<_> = values
        .into_iter()
        .map(
            |(key, label, severity, weight, explanation)| SnapshotFactor {
                key,
                label,
                severity,
                points: severity * weight / total * 100.0,
                explanation,
            },
        )
        .collect();
    let score = factors
        .iter()
        .map(|factor| factor.points)
        .sum::<f64>()
        .round()
        .clamp(0.0, 100.0) as i64;
    SnapshotRisk {
        account_id: account.id.clone(),
        score,
        level: if score >= config.high_threshold {
            "high"
        } else if score >= config.medium_threshold {
            "medium"
        } else {
            "low"
        },
        factors,
        upcoming_cents,
        monthly_cents,
        topups_cents: account.total_replenished_cents,
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
    let rows = sqlx::query_as::<_, Interest>(
        "SELECT i.brand_code, b.name, i.observed_on, i.score::float8 AS score, \
         c.change_7d::float8 AS change_7d, c.change_30d::float8 AS change_30d \
         FROM market_intelligence.search_interest i \
         JOIN market_intelligence.search_brands b ON b.code = i.brand_code \
         LEFT JOIN market_intelligence.search_interest_changes c \
           ON c.brand_code = i.brand_code AND c.source = i.source \
           AND c.observed_on = i.observed_on \
         WHERE i.source = 'google search' \
         ORDER BY b.name, i.observed_on",
    )
    .fetch_all(&pool)
    .await
    .map_err(|error| {
        eprintln!("search-interest query failed: {error}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_json_matches_frontend_field_names() {
        let transaction = ClientTransaction {
            id: "1".into(),
            account_id: "2".into(),
            service_id: "3".into(),
            amount_cents: 500,
            kind: "payment",
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
    fn snapshot_risk_uses_server_data_and_settings() {
        let settings: SnapshotSettings = serde_json::from_str(r#"{"weights":{"balance":1,"renewal":1,"autoRenew":1,"status":1,"transactions":1},"mediumThreshold":40,"highThreshold":70,"renewalWindowDays":7,"historicalMinDays":30}"#).unwrap();
        let now = DateTime::parse_from_rfc3339("2026-10-24T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let account = ClientAccount {
            id: "1".into(),
            name: "Fixture".into(),
            email: "fixture@example.test".into(),
            balance_cents: 0,
            total_replenished_cents: 500,
        };
        let data = ClientDataset {
            accounts: vec![],
            services: vec![ClientService {
                id: "1".into(),
                account_id: "1".into(),
                name: "VPS".into(),
                region: None,
                price_cents: 1000,
                renewal_price_cents: 1000,
                payment_term_seconds: 2_592_000.0,
                renews_at: now,
                auto_renew: false,
                status: "suspended",
            }],
            transactions: vec![ClientTransaction {
                id: "1".into(),
                account_id: "1".into(),
                service_id: "1".into(),
                amount_cents: 1000,
                kind: "payment",
                status: "failed",
                created_at: now,
            }],
            as_of: now,
            snapshots: vec![],
        };
        let risk = calculate_snapshot_risk(&account, &data, &settings);
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
        let now = DateTime::parse_from_rfc3339("2026-10-24T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let account = ClientAccount {
            id: "-1".into(),
            name: "Empty".into(),
            email: "empty@example.test".into(),
            balance_cents: 0,
            total_replenished_cents: 0,
        };
        let data = ClientDataset {
            accounts: vec![],
            services: vec![],
            transactions: vec![],
            as_of: now,
            snapshots: vec![],
        };
        let settings: SnapshotSettings = serde_json::from_str(r#"{"weights":{"balance":35,"renewal":25,"autoRenew":15,"status":15,"transactions":10},"mediumThreshold":35,"highThreshold":65,"renewalWindowDays":14}"#).unwrap();
        let risk = calculate_snapshot_risk(&account, &data, &settings);
        assert_eq!(risk.score, 0);
        assert_eq!(risk.level, "low");
        assert_eq!(risk.upcoming_cents, 0);
        assert!(risk.factors.iter().all(|factor| factor.severity == 0.0));
    }
}
