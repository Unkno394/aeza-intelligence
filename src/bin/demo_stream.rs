//! A bounded, repeatable stream of synthetic customer events for live demos.
//! All owned rows use negative IDs and demo-stream-*@example.test addresses.

use std::{collections::BTreeMap, time::Duration};

use aeza_intelligence::historical::{BehaviorEvent, HistoricalConfig, HistoricalModel};
use anyhow::{Context, Result, ensure};
use clap::Parser;
use sqlx::{PgPool, Row, postgres::PgPoolOptions};

const ACCOUNT_BASE: i64 = -8_500_000;
const SERVICE_BASE: i64 = -8_600_000;
const SERVICES_PER_CUSTOMER: i64 = 3;

#[derive(Parser)]
#[command(about = "Continuously emit isolated synthetic customer events")]
struct Args {
    #[arg(long, env = "DATABASE_URL", hide_env_values = true)]
    database_url: String,
    /// Seconds between changes; 15 is useful for a live demo.
    #[arg(long, env = "DEMO_STREAM_INTERVAL_SECONDS", default_value_t = 15)]
    interval_seconds: u64,
    /// Number of isolated fixture clients to keep in the stream.
    #[arg(long, env = "DEMO_STREAM_CUSTOMERS", default_value_t = 120)]
    customers: i64,
    /// Number of different fixture clients updated at each tick.
    #[arg(long, env = "DEMO_STREAM_BATCH_SIZE", default_value_t = 12)]
    batch_size: i64,
    /// Seed demo rows and emit one change, then exit.
    #[arg(long)]
    once: bool,
    /// Build a synthetic daily Historical Model profile before live events.
    #[arg(long, env = "DEMO_REPLAY_DAYS", default_value_t = 0)]
    replay_days: u32,
    /// Wall-clock pause per virtual day while replaying history.
    #[arg(long, env = "DEMO_REPLAY_DELAY_MS", default_value_t = 200)]
    replay_delay_ms: u64,
    /// Exit after historical replay instead of starting the live stream.
    #[arg(long)]
    replay_only: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    ensure!(
        (2..=3600).contains(&args.interval_seconds),
        "interval must be 2–3600 seconds"
    );
    ensure!(
        (1..=500).contains(&args.customers),
        "customers must be 1–500"
    );
    ensure!(
        (1..=args.customers).contains(&args.batch_size),
        "batch size must be 1–customers"
    );
    ensure!(
        args.replay_days == 0 || (31..=365).contains(&args.replay_days),
        "replay days must be 31–365"
    );
    ensure!(
        args.replay_delay_ms <= 10_000,
        "replay delay must be at most 10000 ms"
    );
    ensure!(
        !args.replay_only || args.replay_days > 0,
        "--replay-only requires --replay-days"
    );
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect(&args.database_url)
        .await?;
    seed(&pool, args.customers).await?;
    if args.replay_days > 0 {
        replay(
            &pool,
            args.replay_days,
            args.replay_delay_ms,
            args.customers,
        )
        .await?;
    }
    if args.replay_only {
        return Ok(());
    }
    let mut tick = tokio::time::interval(Duration::from_secs(args.interval_seconds));
    loop {
        tick.tick().await;
        let slot = chrono::Utc::now()
            .timestamp()
            .div_euclid(args.interval_seconds as i64);
        for offset in 0..args.batch_size {
            let position = slot * args.batch_size + offset;
            let customer = position.rem_euclid(args.customers);
            let phase = stream_phase(position, customer, args.customers);
            emit(&pool, customer, phase).await?;
        }
        eprintln!(
            "demo stream: updated {} of {} fixture clients",
            args.batch_size, args.customers
        );
        if args.once {
            break;
        }
    }
    Ok(())
}

async fn replay(pool: &PgPool, days: u32, delay_ms: u64, customers: i64) -> Result<()> {
    let settings_row = sqlx::query("SELECT version, settings FROM public.client_settings WHERE id")
        .fetch_optional(pool)
        .await?;
    let settings_version = settings_row
        .as_ref()
        .map_or(0, |row| row.get::<i64, _>("version"));
    let settings = settings_row
        .as_ref()
        .map(|row| row.get::<serde_json::Value, _>("settings"));
    let historical_min_days = settings
        .as_ref()
        .and_then(|value| value["historicalMinDays"].as_i64())
        .unwrap_or(30)
        .clamp(7, 365);
    // Leave today free for the real collector to add a snapshot from live data.
    let last_historical_day = chrono::Utc::now().date_naive() - chrono::Duration::days(1);
    let first_historical_day = last_historical_day - chrono::Duration::days(i64::from(days - 1));
    let total_existing: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM public.client_snapshots s JOIN public.accounts a ON a.id = s.account_id \
         WHERE s.account_id BETWEEN $1 AND $2 AND a.email LIKE 'demo-stream-%@example.test' \
         AND s.observed_on BETWEEN $3 AND $4 AND s.settings_version = $5",
    )
    .bind(ACCOUNT_BASE - customers)
    .bind(ACCOUNT_BASE - 1)
    .bind(first_historical_day)
    .bind(last_historical_day)
    .bind(settings_version)
    .fetch_one(pool)
    .await?;
    if total_existing >= i64::from(days) * customers {
        eprintln!("demo replay: {total_existing} snapshots already exist; skipping");
        return Ok(());
    }
    sqlx::query(
        "DELETE FROM public.client_snapshots s USING public.accounts a \
         WHERE s.account_id = a.id AND s.account_id BETWEEN $1 AND $2 \
         AND a.email LIKE 'demo-stream-%@example.test' AND s.observed_on <= $3",
    )
    .bind(ACCOUNT_BASE - customers)
    .bind(ACCOUNT_BASE - 1)
    .bind(last_historical_day)
    .execute(pool)
    .await?;
    let cursor: i64 = sqlx::query_scalar("SELECT COALESCE(max(id), 0) FROM public.operation_logs")
        .fetch_one(pool)
        .await?;
    let mut models: Vec<HistoricalModel> = (0..customers)
        .map(|_| {
            HistoricalModel::new(HistoricalConfig {
                minimum_history_ms: historical_min_days * 86_400_000,
                ..HistoricalConfig::default()
            })
        })
        .collect();
    for day in 0..days {
        let date = last_historical_day - chrono::Duration::days(i64::from(days - day - 1));
        let observed_at = date.and_hms_opt(0, 0, 0).expect("valid midnight").and_utc();
        let mut tx = pool.begin().await?;
        for customer in 0..customers {
            let account_id = ACCOUNT_BASE - customer - 1;
            let profile = customer.rem_euclid(3);
            let stress_days = match profile {
                0 => 10,
                1 => 7,
                _ => 3,
            };
            let stressed = stress_days > 0 && day >= days - stress_days;
            let features = replay_features(day, customer, stressed);
            let model = &mut models[customer as usize];
            let stress_start = days - stress_days;
            let event = if day == 0 {
                Some("active")
            } else if profile != 1 && day == stress_start {
                Some("auto_renew_off")
            } else if profile == 2 && day == stress_start + 1 {
                Some("prolong_wait")
            } else if profile == 2 && day == stress_start + 2 {
                Some("prolong_outcome_unknown")
            } else {
                None
            };
            if let Some(kind) = event {
                let stream = format!(
                    "service:{}",
                    SERVICE_BASE - customer * SERVICES_PER_CUSTOMER - 1
                );
                model.observe_with_event(
                    observed_at.timestamp_millis(),
                    &BTreeMap::new(),
                    Some(BehaviorEvent {
                        stream_id: &stream,
                        kind,
                    }),
                );
            }
            let assessment = model.observe(observed_at.timestamp_millis(), &features, None);
            let assessment_json = mark_synthetic_replay(
                serde_json::to_value(&assessment)?,
                replay_risk_score(&features, settings.as_ref()),
            );
            // The next real daily collector starts after the historical replay,
            // so initial fixture inserts are not counted again as behavior.
            model.set_event_log_cursor(cursor);
            sqlx::query(
                "INSERT INTO public.client_snapshots (account_id, observed_on, observed_at, features, assessment, model_state, settings_version) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7) \
                 ON CONFLICT (account_id, observed_on) DO UPDATE SET \
                 observed_at = EXCLUDED.observed_at, features = EXCLUDED.features, \
                 assessment = EXCLUDED.assessment, model_state = EXCLUDED.model_state, \
                 settings_version = EXCLUDED.settings_version",
            )
            .bind(account_id).bind(date).bind(observed_at)
            .bind(serde_json::to_value(&features)?)
            .bind(assessment_json)
            .bind(serde_json::from_str::<serde_json::Value>(&model.to_json()?)?)
            .bind(settings_version).execute(&mut *tx).await?;
        }
        // A harmless fixture-only account write signals the existing live
        // channel to reload the Historical page during accelerated replay.
        sqlx::query("UPDATE public.accounts SET balance = balance WHERE id = $1")
            .bind(ACCOUNT_BASE - 1)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        if delay_ms > 0 {
            tokio::time::sleep(Duration::from_millis(delay_ms)).await;
        }
    }
    eprintln!("demo replay: built {days} daily snapshots for {customers} fixture clients");
    Ok(())
}

fn replay_features(day: u32, customer: i64, stressed: bool) -> BTreeMap<String, f64> {
    let profile = customer.rem_euclid(3);
    let ordinary_variation = (day % 7) as f64 * 0.01;
    BTreeMap::from([
        ("services_count".into(), 3.0),
        (
            "monthly_spend".into(),
            if stressed {
                45.0
            } else {
                30.0 + ordinary_variation
            },
        ),
        (
            "spendable_balance".into(),
            if stressed && profile != 0 {
                300.0
            } else {
                3000.0
            },
        ),
        (
            "upcoming_payments".into(),
            if stressed { 3000.0 } else { 1600.0 },
        ),
        (
            "payment_coverage".into(),
            if stressed && profile != 0 { 0.1 } else { 1.875 },
        ),
        (
            "no_auto_prolong_share".into(),
            if stressed && profile != 1 { 0.67 } else { 0.0 },
        ),
        (
            "prolong_wait_share".into(),
            if stressed && profile == 2 { 0.33 } else { 0.0 },
        ),
        (
            "expiring_revenue_share".into(),
            if stressed { 0.6 } else { 0.0 },
        ),
    ])
}

fn replay_risk_score(
    features: &BTreeMap<String, f64>,
    settings: Option<&serde_json::Value>,
) -> i64 {
    // Same weighted-severity rule as the live Snapshot API. Unknown renewal
    // outcomes are absent rather than treated as confirmed failures.
    let factors = [
        ("no_auto_prolong_share", "noAutoProlong", 15.0),
        ("prolong_wait_share", "prolongWait", 15.0),
        ("failed_prolong_share", "failedProlong", 10.0),
        ("expiring_revenue_share", "expiringRevenue", 25.0),
        ("payment_coverage", "paymentCoverage", 35.0),
    ];
    let (weighted, total) =
        factors
            .into_iter()
            .fold((0.0, 0.0), |(sum, total), (feature, key, default)| {
                let Some(value) = features
                    .get(feature)
                    .copied()
                    .filter(|value| value.is_finite())
                else {
                    return (sum, total);
                };
                let weight = settings
                    .and_then(|value| value["weights"][key].as_f64())
                    .unwrap_or(default);
                let severity = if feature == "payment_coverage" {
                    1.0 - value.clamp(0.0, 1.0)
                } else {
                    value.clamp(0.0, 1.0)
                };
                (sum + severity * weight, total + weight)
            });
    if total <= 0.0 {
        0
    } else {
        (weighted / total * 100.0).round().clamp(0.0, 100.0) as i64
    }
}

fn mark_synthetic_replay(
    mut assessment: serde_json::Value,
    snapshot_risk_score: i64,
) -> serde_json::Value {
    assessment["snapshot_risk_score"] = serde_json::json!(snapshot_risk_score);
    assessment["synthetic_replay"] = serde_json::json!(true);
    assessment
}

async fn seed(pool: &PgPool, customers: i64) -> Result<()> {
    let mut tx = pool.begin().await?;
    for customer in 0..customers {
        let account_id = ACCOUNT_BASE - customer - 1;
        let email = format!("demo-stream-{}@example.test", customer + 1);
        sqlx::query(
            "INSERT INTO public.accounts (id, email, balance, \"withdrawBalance\", \"totalReplenished\", \"bonusBalance\", \"bonusState\") \
             VALUES ($1, $2, 3000, 0, 15000, 0, 'locked') ON CONFLICT (id) DO NOTHING",
        )
        .bind(account_id).bind(&email).execute(&mut *tx).await?;
        let actual: Option<String> =
            sqlx::query_scalar("SELECT email FROM public.accounts WHERE id = $1")
                .bind(account_id)
                .fetch_optional(&mut *tx)
                .await?;
        ensure!(
            actual.as_deref() == Some(email.as_str()),
            "demo account ID {account_id} belongs to another record"
        );
        for service in 0..SERVICES_PER_CUSTOMER {
            let service_id = SERVICE_BASE - customer * SERVICES_PER_CUSTOMER - service - 1;
            let name = format!("demo-stream-{}-{}", customer + 1, service + 1);
            sqlx::query(
                "INSERT INTO public.services (id, name, \"ownerId\", \"paymentTerm\", \"autoProlong\", \"createdAt\", \"expiresAt\", status, price) \
                 VALUES ($1, $2, $3, 'month', true, now() - interval '60 days', now() + interval '14 days', 'active', $4) \
                 ON CONFLICT (id) DO NOTHING",
            )
            .bind(service_id).bind(&name).bind(account_id).bind(800 + service * 200)
            .execute(&mut *tx).await?;
            let row = sqlx::query(
                "SELECT name, \"ownerId\"::bigint AS owner_id FROM public.services WHERE id = $1",
            )
            .bind(service_id)
            .fetch_one(&mut *tx)
            .await?;
            ensure!(
                row.get::<String, _>("name") == name && row.get::<i64, _>("owner_id") == account_id,
                "demo service ID {service_id} belongs to another record"
            );
        }
    }
    tx.commit().await?;
    Ok(())
}

async fn emit(pool: &PgPool, customer: i64, phase: i64) -> Result<()> {
    let account_id = ACCOUNT_BASE - customer - 1;
    let service_id = SERVICE_BASE - customer * SERVICES_PER_CUSTOMER - 1;
    let profile = customer.rem_euclid(3);
    let mut tx = pool.begin().await?;
    // The event log triggers capture each change; this transaction makes each
    // demo step visible to Risk Score as one consistent state.
    match phase {
        0 => {
            sqlx::query("UPDATE public.accounts SET balance = 3000, \"totalReplenished\" = \"totalReplenished\" + GREATEST(3000 - balance, 0) WHERE id = $1")
                .bind(account_id).execute(&mut *tx).await?;
            sqlx::query("UPDATE public.services SET \"autoProlong\" = true, status = 'active', \"expiresAt\" = now() + interval '14 days' WHERE \"ownerId\" = $1")
                .bind(account_id).execute(&mut *tx).await?;
            sqlx::query("UPDATE public.services SET price = 800 WHERE id = $1")
                .bind(service_id)
                .execute(&mut *tx)
                .await?;
        }
        1 => {
            sqlx::query("UPDATE public.services SET \"autoProlong\" = $2, \"expiresAt\" = now() + interval '2 days' WHERE id = $1")
                .bind(service_id).bind(profile == 1).execute(&mut *tx).await?;
        }
        2 => {
            if profile != 0 {
                sqlx::query("UPDATE public.accounts SET balance = 300 WHERE id = $1")
                    .bind(account_id)
                    .execute(&mut *tx)
                    .await?;
                for service in 0..SERVICES_PER_CUSTOMER {
                    let paid_service =
                        SERVICE_BASE - customer * SERVICES_PER_CUSTOMER - service - 1;
                    sqlx::query("INSERT INTO public.transactions (amount, status, \"createdAt\", \"performedAt\", type, \"serviceId\") VALUES (900, 'performed', now(), now(), 'prolong', $1)")
                        .bind(paid_service).execute(&mut *tx).await?;
                }
            }
            sqlx::query("UPDATE public.services SET \"autoProlong\" = $2, \"expiresAt\" = now() + interval '1 day' WHERE \"ownerId\" = $1")
                .bind(account_id).bind(profile == 1).execute(&mut *tx).await?;
        }
        3 => {
            if profile != 0 {
                sqlx::query("UPDATE public.services SET status = 'prolong_wait', \"expiresAt\" = now() - interval '1 day' WHERE id = $1")
                    .bind(service_id).execute(&mut *tx).await?;
            }
            if profile == 2 {
                sqlx::query("INSERT INTO public.transactions (amount, status, \"createdAt\", type, \"serviceId\") VALUES (800, 'not_performed', now(), 'prolong', $1)")
                    .bind(service_id).execute(&mut *tx).await?;
            }
        }
        4 => {
            sqlx::query("UPDATE public.accounts SET balance = $2, \"totalReplenished\" = \"totalReplenished\" + GREATEST(3000 - balance, 0) WHERE id = $1")
                .bind(account_id).bind(if profile == 0 { 3000 } else { 2200 }).execute(&mut *tx).await?;
            sqlx::query("UPDATE public.services SET status = 'active', \"autoProlong\" = true, \"expiresAt\" = now() + interval '30 days' WHERE \"ownerId\" = $1")
                .bind(account_id).execute(&mut *tx).await?;
            if profile != 0 {
                sqlx::query("INSERT INTO public.transactions (amount, status, \"createdAt\", \"performedAt\", type, \"serviceId\") VALUES (800, 'performed', now(), now(), 'prolong', $1)")
                    .bind(service_id).execute(&mut *tx).await?;
            }
        }
        5 => {
            // A synthetic price change is logged by the ordinary service trigger.
            // Real price-risk metrics deliberately exclude negative fixture IDs.
            sqlx::query("UPDATE public.services SET price = price + 100 WHERE id = $1")
                .bind(service_id)
                .execute(&mut *tx)
                .await?;
        }
        _ => anyhow::bail!("invalid demo phase"),
    }
    tx.commit()
        .await
        .context("could not commit synthetic event")?;
    Ok(())
}

/// Stagger each client's six-step demo cycle so the snapshot always contains
/// healthy, warning, and high-risk profiles instead of changing every client
/// through the same state at once.
fn stream_phase(position: i64, customer: i64, customers: i64) -> i64 {
    (position.div_euclid(customers) + customer).rem_euclid(6)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn batched_ticks_visit_every_client_with_staggered_phases() {
        let customers: i64 = 120;
        let batch_size: i64 = 12;
        let first_cycle: Vec<_> = (0..10)
            .flat_map(|slot| {
                (0..batch_size).map(move |offset| {
                    let position = slot * batch_size + offset;
                    let customer = position.rem_euclid(customers);
                    (customer, stream_phase(position, customer, customers))
                })
            })
            .collect();
        assert_eq!(first_cycle.len(), customers as usize);
        for customer in 0..customers {
            assert_eq!(
                first_cycle.iter().filter(|(id, _)| *id == customer).count(),
                1
            );
        }
        let mut phases = [0; 6];
        for (customer, phase) in &first_cycle {
            assert_eq!(*phase, customer.rem_euclid(6));
            phases[*phase as usize] += 1;
        }
        assert!(phases.iter().all(|count| *count == customers as usize / 6));
        for position in customers..customers * 2 {
            let customer = position.rem_euclid(customers);
            assert_eq!(
                stream_phase(position, customer, customers),
                (customer + 1).rem_euclid(6)
            );
        }
        for profile in 0..3 {
            assert_eq!(
                replay_features(50, profile, true),
                replay_features(50, profile + 3, true)
            );
        }
    }

    #[test]
    fn replay_score_tracks_stress_and_saved_weights() {
        assert_eq!(replay_risk_score(&replay_features(20, 0, false), None), 0);
        assert_eq!(replay_risk_score(&replay_features(50, 0, true), None), 28);
        assert_eq!(replay_risk_score(&replay_features(50, 1, true), None), 52);
        assert_eq!(replay_risk_score(&replay_features(50, 2, true), None), 68);
        let changed = serde_json::json!({"weights": {"paymentCoverage": 80.0}});
        assert_eq!(
            replay_risk_score(&replay_features(50, 2, true), Some(&changed)),
            76
        );
        assert!(!replay_features(50, 2, true).contains_key("failed_prolong_share"));
    }

    #[test]
    fn synthetic_replay_is_marked_without_changing_historical_facts() {
        let historical_facts = serde_json::json!({
            "is_confident": true,
            "is_unusual": true,
            "anomaly_score": 4.2,
            "observations": 120
        });
        let marked = mark_synthetic_replay(historical_facts.clone(), 87);

        assert_eq!(marked["synthetic_replay"], true);
        assert_eq!(marked["snapshot_risk_score"], 87);
        assert_eq!(marked["is_confident"], historical_facts["is_confident"]);
        assert_eq!(marked["is_unusual"], historical_facts["is_unusual"]);
        assert_eq!(marked["anomaly_score"], historical_facts["anomaly_score"]);
        assert_eq!(marked["observations"], historical_facts["observations"]);
        assert!(historical_facts.get("synthetic_replay").is_none());
    }
}
