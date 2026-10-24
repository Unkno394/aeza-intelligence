//! Offline integration check for the Rust snapshot and historical models.
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};

use aeza_intelligence::{
    PgAdapter,
    analysis::{self, Params},
    historical::HistoricalModel,
    model::ClientFeatures,
};
use anyhow::{Context, Result, ensure};
use chrono::{DateTime, Utc};
use clap::Parser;
use serde::{Deserialize, Serialize};
use sqlx::Row;

#[derive(Parser)]
struct Cli {
    path: PathBuf,
    /// Write generated history only for matching example.test accounts already in PostgreSQL.
    #[arg(long)]
    persist: bool,
    #[arg(long, env = "DATABASE_URL", hide_env_values = true)]
    database_url: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Day {
    observed_at: DateTime<Utc>,
    clients: Vec<ClientFeatures>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Summary {
    days: usize,
    accounts: usize,
    snapshot_clients: usize,
    snapshot_features: usize,
    historical_confident: usize,
    historical_unusual: Vec<i64>,
    first_account_familiarity: f64,
    persisted_snapshots: u64,
}

struct SavedSnapshot {
    account_id: i64,
    email: String,
    observed_at: DateTime<Utc>,
    features: String,
    assessment: String,
    model_state: String,
}

fn feature_map(client: &ClientFeatures) -> BTreeMap<String, f64> {
    let mut values = BTreeMap::from([
        ("services_count".into(), client.services_count as f64),
        ("monthly_spend".into(), client.monthly_spend),
        ("spendable_balance".into(), client.spendable_balance as f64),
        ("upcoming_payments".into(), client.upcoming_payments as f64),
    ]);
    if let Some(coverage) = client.payment_coverage.filter(|value| value.is_finite()) {
        values.insert("payment_coverage".into(), coverage);
    }
    for (name, value) in [
        ("no_auto_prolong_share", client.no_auto_prolong_share),
        ("prolong_wait_share", client.prolong_wait_share),
        ("failed_prolong_share", client.failed_prolong_share),
        ("expiring_revenue_share", client.expiring_revenue_share),
    ] {
        if let Some(value) = value.filter(|value| value.is_finite()) {
            values.insert(name.into(), value);
        }
    }
    values
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let path = cli.path;
    let days: Vec<Day> = serde_json::from_slice(
        &std::fs::read(&path).with_context(|| format!("cannot read {}", path.display()))?,
    )?;
    ensure!(
        days.len() >= 31,
        "at least 31 dated observations are required"
    );
    let expected: BTreeSet<i64> = days[0]
        .clients
        .iter()
        .map(|client| client.account_id)
        .collect();
    let changed_account = days[0].clients[0].account_id;
    ensure!(
        !expected.is_empty() && expected.len() == days[0].clients.len(),
        "fixture account IDs must be unique"
    );
    let mut models: BTreeMap<i64, HistoricalModel> = expected
        .iter()
        .map(|id| (*id, HistoricalModel::default()))
        .collect();
    let mut last_assessments = BTreeMap::new();
    let mut snapshots = Vec::new();
    for (index, day) in days.iter().enumerate() {
        if index > 0 {
            ensure!(
                day.observed_at > days[index - 1].observed_at,
                "history dates must increase"
            );
        }
        let actual: BTreeSet<_> = day.clients.iter().map(|client| client.account_id).collect();
        ensure!(
            actual == expected && actual.len() == day.clients.len(),
            "account IDs changed or repeated on day {index}"
        );
        for client in &day.clients {
            let model = models
                .get_mut(&client.account_id)
                .expect("validated account ID");
            let assessment = model.observe(
                day.observed_at.timestamp_millis(),
                &feature_map(client),
                None,
            );
            if cli.persist {
                snapshots.push(SavedSnapshot {
                    account_id: client.account_id,
                    email: client.email.clone(),
                    observed_at: day.observed_at,
                    features: serde_json::to_string(&feature_map(client))?,
                    assessment: serde_json::to_string(&assessment)?,
                    model_state: model.to_json()?,
                });
            }
            last_assessments.insert(client.account_id, assessment);
        }
        if index == days.len() / 2 {
            for model in models.values_mut() {
                *model = HistoricalModel::from_json(&model.to_json()?)?;
            }
        }
    }
    let report = analysis::analyze(
        &days.last().unwrap().clients,
        Params {
            k: 3,
            shrinkage: 0.3,
        },
    );
    ensure!(
        report.clients.len() == expected.len() && report.features.len() == 5,
        "snapshot model returned incomplete output"
    );
    ensure!(
        report
            .clients
            .iter()
            .all(|client| client.robust_z.iter().all(|value| value.is_finite())),
        "snapshot model returned a non-finite score"
    );
    let unusual: Vec<i64> = last_assessments
        .iter()
        .filter_map(|(id, assessment)| assessment.is_unusual.then_some(*id))
        .collect();
    let confident = last_assessments
        .values()
        .filter(|assessment| assessment.is_confident)
        .count();
    ensure!(
        confident == expected.len(),
        "historical model did not become confident after {} days",
        days.len()
    );
    ensure!(
        unusual.contains(&changed_account),
        "historical model missed the generated change for account {changed_account}"
    );
    let persisted_snapshots = if cli.persist {
        let url = cli
            .database_url
            .as_deref()
            .context("DATABASE_URL is required with --persist")?;
        persist(&snapshots, url).await?
    } else {
        0
    };
    let result = Summary {
        days: days.len(),
        accounts: expected.len(),
        snapshot_clients: report.clients.len(),
        snapshot_features: report.features.len(),
        historical_confident: confident,
        historical_unusual: unusual,
        first_account_familiarity: last_assessments[&changed_account].familiarity,
        persisted_snapshots,
    };
    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(())
}

async fn persist(snapshots: &[SavedSnapshot], database_url: &str) -> Result<u64> {
    let adapter = PgAdapter::connect(database_url).await?;
    adapter.migrate().await?;
    let expected: BTreeMap<i64, &str> = snapshots
        .iter()
        .map(|row| (row.account_id, row.email.as_str()))
        .collect();
    ensure!(
        expected
            .values()
            .all(|email| email.ends_with("@example.test")),
        "fixture accounts must use example.test addresses"
    );
    let rows =
        sqlx::query("SELECT id::bigint AS id, email FROM public.accounts WHERE id = ANY($1)")
            .bind(expected.keys().copied().collect::<Vec<_>>())
            .fetch_all(adapter.pool())
            .await?;
    let found: BTreeMap<i64, String> = rows
        .into_iter()
        .map(|row| (row.get("id"), row.get("email")))
        .collect();
    ensure!(
        found.len() == expected.len()
            && expected
                .iter()
                .all(|(id, email)| found.get(id).is_some_and(|actual| actual == email)),
        "database accounts do not match generated fixture IDs and emails; no history was written"
    );
    let event_highwater: i64 =
        sqlx::query_scalar("SELECT COALESCE(MAX(id), 0) FROM public.operation_logs")
            .fetch_one(adapter.pool())
            .await?;
    let mut tx = adapter.pool().begin().await?;
    let mut inserted = 0;
    for row in snapshots {
        let mut model_state: serde_json::Value = serde_json::from_str(&row.model_state)?;
        model_state["event_log_cursor"] = serde_json::json!(event_highwater);
        let result = sqlx::query("INSERT INTO public.client_snapshots (account_id, observed_on, observed_at, features, assessment, model_state) VALUES ($1, $2, $3, $4::jsonb, $5::jsonb, $6::jsonb) ON CONFLICT (account_id, observed_on) DO NOTHING")
            .bind(row.account_id).bind(row.observed_at.date_naive()).bind(row.observed_at)
            .bind(&row.features).bind(&row.assessment).bind(serde_json::to_string(&model_state)?)
            .execute(&mut *tx).await?;
        inserted += result.rows_affected();
    }
    tx.commit().await?;
    Ok(inserted)
}
