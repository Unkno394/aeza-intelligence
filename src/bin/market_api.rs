use std::{
    collections::BTreeMap,
    net::SocketAddr,
    sync::Arc,
    time::{Duration, Instant},
};

use aeza_intelligence::{
    PgAdapter,
    analysis::{self, Params},
};
use anyhow::Result;
use axum::{
    Json, Router,
    extract::{
        FromRef, Path, State,
        ws::{Message, Utf8Bytes, WebSocket, WebSocketUpgrade},
    },
    http::StatusCode,
    response::Response,
    routing::{get, post},
};
use chrono::{DateTime, NaiveDate, Utc};
use clap::Parser;
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row, postgres::PgPoolOptions};
use tokio::sync::{Mutex, broadcast};

/// How often the live channel checks operation_logs for client data changes.
const POLL_INTERVAL: Duration = Duration::from_secs(2);
/// Scores depend on the current time (renewal window), so they are recalculated
/// at least this often even without data changes.
const RESCORE_INTERVAL: Duration = Duration::from_secs(60);
const PING_INTERVAL: Duration = Duration::from_secs(30);

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

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SnapshotSettings {
    weights: SnapshotWeights,
    medium_threshold: i64,
    high_threshold: i64,
    renewal_window_days: i64,
    /// Used only by the frontend Historical page; stored with the other settings.
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
                balance: 35.0,
                renewal: 25.0,
                auto_renew: 15.0,
                status: 15.0,
                transactions: 10.0,
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
            weights.balance,
            weights.renewal,
            weights.auto_renew,
            weights.status,
            weights.transactions,
        ];
        all.iter()
            .all(|weight| weight.is_finite() && *weight >= 0.0)
            && all.iter().sum::<f64>() > 0.0
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
    balance: f64,
    renewal: f64,
    auto_renew: f64,
    status: f64,
    transactions: f64,
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
}

#[derive(Serialize, PartialEq)]
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
    let live = Live::start(pool.clone()).await?;
    let app = Router::new()
        .route("/api/market/search-interest", get(search_interest))
        .route("/api/market/geography", get(geography))
        .route("/api/market/prices", get(prices))
        .route("/api/client/dataset", get(client_dataset))
        .route("/api/client/analysis", get(client_analysis))
        .route("/api/client/snapshot", post(client_snapshot_risks))
        .route("/api/client/historical/{id}", get(client_history))
        .route("/api/client/ws", get(live_socket))
        .with_state(AppState { pool, live });
    let listener = tokio::net::TcpListener::bind(cli.listen).await?;
    eprintln!("market API listening on http://{}", cli.listen);
    axum::serve(listener, app).await?;
    Ok(())
}

#[derive(Clone)]
struct AppState {
    pool: PgPool,
    live: Arc<Live>,
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
        risks: Vec<&'a SnapshotRisk>,
    },
    /// Accounts, services or transactions changed.
    Dataset {
        dataset: &'a ClientDataset,
    },
    Settings {
        settings: &'a SnapshotSettings,
    },
    /// Only the accounts whose Risk Score changed.
    Scores {
        changed: Vec<&'a SnapshotRisk>,
        removed: Vec<&'a str>,
    },
    Ack {
        request_id: &'a str,
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

/// Shared client state for every WebSocket session: the latest dataset, the
/// settings and the Risk Score per account. Changes are broadcast as encoded
/// messages.
struct Live {
    events: broadcast::Sender<Utf8Bytes>,
    state: Mutex<LiveState>,
}

struct LiveState {
    settings: SnapshotSettings,
    dataset: ClientDataset,
    risks: BTreeMap<String, SnapshotRisk>,
    /// Latest operation_logs id: every write to the client tables appends a row.
    log_id: Option<i64>,
}

impl Live {
    async fn start(pool: PgPool) -> Result<Arc<Self>> {
        let settings = load_settings(&pool).await?;
        let log_id = last_log_id(&pool).await?;
        let dataset = load_dataset(&pool).await?;
        let risks = calculate_risks(&dataset, &settings);
        let live = Arc::new(Self {
            events: broadcast::channel(64).0,
            state: Mutex::new(LiveState {
                settings,
                dataset,
                risks,
                log_id,
            }),
        });
        tokio::spawn(live.clone().watch(pool));
        Ok(live)
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
                Err(error) => eprintln!("live refresh failed: {error}"),
            }
        }
    }

    /// Reloads the dataset when client data changed (or `rescore` is set) and
    /// broadcasts what differs. Returns whether scores were recalculated.
    async fn refresh(&self, pool: &PgPool, rescore: bool) -> Result<bool, sqlx::Error> {
        let log_id = last_log_id(pool).await?;
        let changed = log_id != self.state.lock().await.log_id;
        if !changed && !rescore {
            return Ok(false);
        }
        let dataset = load_dataset(pool).await?;
        let mut state = self.state.lock().await;
        state.dataset = dataset;
        state.log_id = log_id;
        if changed {
            self.publish(&ServerMessage::Dataset {
                dataset: &state.dataset,
            });
        }
        self.rescore(&mut state);
        Ok(true)
    }

    fn rescore(&self, state: &mut LiveState) {
        let risks = calculate_risks(&state.dataset, &state.settings);
        let changed: Vec<_> = risks
            .values()
            .filter(|risk| state.risks.get(&risk.account_id) != Some(*risk))
            .collect();
        let removed: Vec<_> = state
            .risks
            .keys()
            .filter(|id| !risks.contains_key(*id))
            .map(String::as_str)
            .collect();
        if !changed.is_empty() || !removed.is_empty() {
            self.publish(&ServerMessage::Scores { changed, removed });
        }
        state.risks = risks;
    }

    async fn update_settings(
        &self,
        pool: &PgPool,
        settings: SnapshotSettings,
    ) -> Result<(), &'static str> {
        if !settings.is_valid() {
            return Err("Некорректные настройки");
        }
        // Held across the write so concurrent updates reach the database and
        // the sessions in the same order.
        let mut state = self.state.lock().await;
        save_settings(pool, &settings).await.map_err(|error| {
            eprintln!("saving client settings failed: {error}");
            "Не удалось сохранить настройки"
        })?;
        state.settings = settings;
        self.publish(&ServerMessage::Settings {
            settings: &state.settings,
        });
        self.rescore(&mut state);
        Ok(())
    }

    async fn snapshot(&self) -> Utf8Bytes {
        let state = self.state.lock().await;
        encode(&ServerMessage::Snapshot {
            dataset: &state.dataset,
            settings: &state.settings,
            risks: state.risks.values().collect(),
        })
    }

    async fn handle(&self, pool: &PgPool, text: &str) -> Utf8Bytes {
        match serde_json::from_str::<ClientMessage>(text) {
            Ok(ClientMessage::UpdateSettings {
                request_id,
                settings,
            }) => match self.update_settings(pool, settings).await {
                Ok(()) => encode(&ServerMessage::Ack {
                    request_id: &request_id,
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

async fn live_socket(ws: WebSocketUpgrade, State(state): State<AppState>) -> Response {
    ws.on_upgrade(move |socket| serve_socket(socket, state))
}

async fn serve_socket(mut socket: WebSocket, state: AppState) {
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
                Some(Ok(Message::Text(text))) => state.live.handle(&state.pool, &text).await,
                Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break,
                Some(Ok(_)) => continue,
            },
            _ = ping.tick() => {
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

async fn load_settings(pool: &PgPool) -> Result<SnapshotSettings, sqlx::Error> {
    let stored: Option<serde_json::Value> =
        sqlx::query_scalar("SELECT settings FROM public.client_settings WHERE id")
            .fetch_optional(pool)
            .await?;
    Ok(stored
        .and_then(
            |value| match serde_json::from_value::<SnapshotSettings>(value) {
                Ok(settings) if settings.is_valid() => Some(settings),
                _ => {
                    eprintln!("stored client settings are invalid, using defaults");
                    None
                }
            },
        )
        .unwrap_or_default())
}

async fn save_settings(pool: &PgPool, settings: &SnapshotSettings) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO public.client_settings (id, settings) VALUES (true, $1) \
         ON CONFLICT (id) DO UPDATE SET settings = excluded.settings, updated_at = now()",
    )
    .bind(serde_json::to_value(settings).expect("settings always serialize"))
    .execute(pool)
    .await?;
    Ok(())
}

async fn last_log_id(pool: &PgPool) -> Result<Option<i64>, sqlx::Error> {
    sqlx::query_scalar("SELECT max(id) FROM public.operation_logs")
        .fetch_one(pool)
        .await
}

fn calculate_risks(
    dataset: &ClientDataset,
    settings: &SnapshotSettings,
) -> BTreeMap<String, SnapshotRisk> {
    dataset
        .accounts
        .iter()
        .map(|account| {
            (
                account.id.clone(),
                calculate_snapshot_risk(account, dataset, settings),
            )
        })
        .collect()
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
        "SELECT id::bigint AS id, \"ownerId\"::bigint AS owner_id, name, \
         round(price::numeric * 2592000 / public.payment_term_seconds(\"paymentTerm\"::text))::bigint AS monthly_price, \
         price AS renewal_price, public.payment_term_seconds(\"paymentTerm\"::text) AS payment_term_seconds, \
         \"expiresAt\", \"autoProlong\", status::text AS status \
         FROM public.services ORDER BY id",
    )
    .fetch_all(pool)
    .await?;
    let transaction_rows = sqlx::query(
        "SELECT t.id::bigint AS id, s.\"ownerId\"::bigint AS owner_id, \
         t.\"serviceId\"::bigint AS service_id, t.amount::bigint AS amount, \
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
    Ok(ClientDataset {
        accounts,
        services,
        transactions,
        as_of: Utc::now(),
        snapshots: Vec::new(),
    })
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
    let dataset = load_dataset(&pool).await.map_err(db_error)?;
    Ok(Json(
        calculate_risks(&dataset, &settings).into_values().collect(),
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
    fn live_messages_use_frontend_field_names() {
        let message: ClientMessage = serde_json::from_str(r#"{"type":"updateSettings","requestId":"7","settings":{"weights":{"balance":1,"renewal":1,"autoRenew":1,"status":1,"transactions":1},"mediumThreshold":40,"highThreshold":70,"renewalWindowDays":7,"historicalMinDays":30}}"#).unwrap();
        let ClientMessage::UpdateSettings {
            request_id,
            settings,
        } = message;
        assert_eq!(request_id, "7");
        assert!(settings.is_valid());
        let ack: serde_json::Value =
            serde_json::from_str(encode(&ServerMessage::Ack { request_id: "7" }).as_str()).unwrap();
        assert_eq!(ack, serde_json::json!({"type": "ack", "requestId": "7"}));
        let defaults = serde_json::to_value(SnapshotSettings::default()).unwrap();
        assert_eq!(defaults["weights"]["autoRenew"], 15.0);
        assert_eq!(defaults["historicalMinDays"], 30);
        assert!(SnapshotSettings::default().is_valid());
    }

    #[test]
    fn rescore_broadcasts_only_changed_scores() {
        let now = DateTime::parse_from_rfc3339("2026-10-24T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let account = |id: &str| ClientAccount {
            id: id.into(),
            name: id.into(),
            email: format!("{id}@example.test"),
            balance_cents: 0,
            total_replenished_cents: 0,
        };
        let dataset = ClientDataset {
            accounts: vec![account("1"), account("2")],
            services: vec![ClientService {
                id: "1".into(),
                account_id: "1".into(),
                name: "VPS".into(),
                region: None,
                price_cents: 1000,
                renewal_price_cents: 1000,
                payment_term_seconds: 2_592_000.0,
                renews_at: now + chrono::Duration::days(60),
                auto_renew: true,
                status: "suspended",
            }],
            transactions: vec![],
            as_of: now,
            snapshots: vec![],
        };
        let settings = SnapshotSettings::default();
        let live = Live {
            events: broadcast::channel(8).0,
            state: Mutex::new(LiveState {
                risks: calculate_risks(&dataset, &settings),
                settings,
                dataset,
                log_id: None,
            }),
        };
        let mut events = live.events.subscribe();
        let mut state = live.state.try_lock().unwrap();

        live.rescore(&mut state);
        assert!(events.try_recv().is_err(), "nothing changed");

        state.settings.weights.status = 100.0;
        live.rescore(&mut state);
        let message: serde_json::Value =
            serde_json::from_str(events.try_recv().unwrap().as_str()).unwrap();
        assert_eq!(message["type"], "scores");
        let changed = message["changed"].as_array().unwrap();
        assert_eq!(changed.len(), 1);
        assert_eq!(changed[0]["accountId"], "1");
        assert_eq!(state.risks["1"].score, changed[0]["score"]);
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
