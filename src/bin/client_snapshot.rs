use std::{collections::BTreeMap, time::Duration};

use aeza_intelligence::{PgAdapter, historical::HistoricalModel, model::ClientFeatures};
use anyhow::{Context, Result, bail, ensure};
use chrono::{DateTime, Utc};
use clap::Parser;
use sqlx::Row;

#[derive(Parser)]
#[command(about = "Save one daily client feature and historical-model snapshot")]
struct Cli {
    #[arg(long, env = "DATABASE_URL", hide_env_values = true)]
    database_url: String,
    /// Keep checking hourly; each account gets at most one snapshot per UTC day.
    #[arg(long)]
    schedule: bool,
    /// Test one fixture account at a controlled UTC instant.
    #[arg(long)]
    as_of: Option<DateTime<Utc>>,
    #[arg(long, allow_hyphen_values = true)]
    account_id: Option<i64>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    if cli.schedule && (cli.as_of.is_some() || cli.account_id.is_some()) {
        bail!("--schedule cannot be combined with a controlled date or account");
    }
    if cli.as_of.is_some() != cli.account_id.is_some() {
        bail!("--as-of and --account-id must be supplied together");
    }
    let adapter = PgAdapter::connect(&cli.database_url).await?;
    adapter.migrate().await?;
    if let Some(account_id) = cli.account_id {
        ensure!(
            account_id < 0,
            "controlled dates are restricted to negative fixture IDs"
        );
        let email: Option<String> =
            sqlx::query_scalar("SELECT email FROM public.accounts WHERE id = $1")
                .bind(account_id)
                .fetch_optional(adapter.pool())
                .await?;
        ensure!(
            email.as_deref().is_some_and(
                |email| email.starts_with("model-fixture-") && email.ends_with("@example.test")
            ),
            "controlled dates require a generated model fixture account"
        );
    }
    loop {
        let failed =
            match collect_at(&adapter, cli.as_of.unwrap_or_else(Utc::now), cli.account_id).await {
                Ok(count) => {
                    println!("saved {count} client snapshots");
                    false
                }
                Err(error) if cli.schedule => {
                    eprintln!("client snapshot failed: {error:#}; retrying in 60 seconds");
                    true
                }
                Err(error) => return Err(error),
            };
        if !cli.schedule {
            break;
        }
        tokio::time::sleep(Duration::from_secs(if failed { 60 } else { 3600 })).await;
    }
    Ok(())
}

async fn collect_at(
    adapter: &PgAdapter,
    now: DateTime<Utc>,
    account_id: Option<i64>,
) -> Result<usize> {
    let day = now.date_naive();
    let clients = adapter.client_features(now, "7 days").await?;
    let mut count = 0;
    for client in clients
        .into_iter()
        .filter(|client| account_id.is_none_or(|id| client.account_id == id))
    {
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM public.client_snapshots WHERE account_id = $1 AND observed_on = $2)",
        )
        .bind(client.account_id)
        .bind(day)
        .fetch_one(adapter.pool())
        .await?;
        if exists {
            continue;
        }

        let previous = sqlx::query(
            "SELECT model_state::text AS state FROM public.client_snapshots \
             WHERE account_id = $1 AND observed_on < $2 ORDER BY observed_on DESC LIMIT 1",
        )
        .bind(client.account_id)
        .bind(day)
        .fetch_optional(adapter.pool())
        .await?;
        let mut model = match previous {
            Some(row) => HistoricalModel::from_json(row.get::<&str, _>("state"))
                .context("invalid saved historical model")?,
            None => HistoricalModel::default(),
        };
        let features = feature_map(&client);
        let assessment = model.observe(now.timestamp_millis(), &features, None);
        let inserted = sqlx::query(
            "INSERT INTO public.client_snapshots \
             (account_id, observed_on, observed_at, features, assessment, model_state) \
             VALUES ($1, $2, $3, $4::jsonb, $5::jsonb, $6::jsonb) \
             ON CONFLICT (account_id, observed_on) DO NOTHING",
        )
        .bind(client.account_id)
        .bind(day)
        .bind(now)
        .bind(serde_json::to_string(&features)?)
        .bind(serde_json::to_string(&assessment)?)
        .bind(model.to_json()?)
        .execute(adapter.pool())
        .await?;
        count += inserted.rows_affected() as usize;
    }
    Ok(count)
}

fn feature_map(client: &ClientFeatures) -> BTreeMap<String, f64> {
    let mut values = BTreeMap::from([
        ("services_count".into(), client.services_count as f64),
        ("monthly_spend".into(), client.monthly_spend),
        ("payment_coverage".into(), client.payment_coverage),
        ("upcoming_payments".into(), client.upcoming_payments as f64),
    ]);
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
    values.retain(|_, value| value.is_finite());
    values
}
