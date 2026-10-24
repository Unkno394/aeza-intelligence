use std::time::Duration;

use anyhow::{Context, Result, bail};
use chrono::{NaiveDate, Utc};
use clap::Parser;
use reqwest::Client;
use serde::Deserialize;
use sqlx::postgres::PgPoolOptions;

const SCHEMA: &str = include_str!("../../migrations/003_exchange_rates.sql");

#[derive(Parser)]
#[command(about = "Fetch historical EUR/RUB rates for imported competitor prices")]
struct Cli {
    #[arg(long, env = "DATABASE_URL", hide_env_values = true)]
    database_url: Option<String>,
    /// Fetch and parse one rate without using PostgreSQL.
    #[arg(long)]
    dry_run: bool,
    /// Date to inspect in dry-run mode; defaults to today.
    #[arg(long)]
    date: Option<NaiveDate>,
}

#[derive(Deserialize)]
struct Rate {
    date: NaiveDate,
    base: String,
    quote: String,
    rate: f64,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let client = Client::builder().timeout(Duration::from_secs(30)).build()?;
    if cli.dry_run {
        let requested_on = cli.date.unwrap_or_else(|| Utc::now().date_naive());
        let rate = fetch_rate(&client, requested_on).await?;
        println!(
            "requested_on={requested_on} rate_on={} EUR/RUB={}",
            rate.date, rate.rate
        );
        return Ok(());
    }
    let database_url = cli
        .database_url
        .context("DATABASE_URL is required unless --dry-run is used")?;
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect(&database_url)
        .await?;
    sqlx::raw_sql(SCHEMA).execute(&pool).await?;
    let dates: Vec<NaiveDate> = sqlx::query_scalar(
        "SELECT DISTINCT p.observed_at::date FROM market_intelligence.tariff_prices p \
         WHERE p.currency = 'EUR' AND p.price_minor IS NOT NULL \
         AND NOT EXISTS (SELECT 1 FROM market_intelligence.exchange_rates fx \
           WHERE fx.base = 'EUR' AND fx.quote = 'RUB' AND fx.requested_on = p.observed_at::date) \
         ORDER BY 1",
    )
    .fetch_all(&pool)
    .await?;
    let mut failures = 0;
    for requested_on in dates {
        match fetch_rate(&client, requested_on).await {
            Ok(rate) => {
                sqlx::query(
                    "INSERT INTO market_intelligence.exchange_rates \
                     (base, quote, requested_on, rate_on, rate) VALUES ('EUR', 'RUB', $1, $2, $3) \
                     ON CONFLICT (base, quote, requested_on) DO UPDATE SET \
                     rate_on = EXCLUDED.rate_on, rate = EXCLUDED.rate, fetched_at = now()",
                )
                .bind(requested_on)
                .bind(rate.date)
                .bind(rate.rate)
                .execute(&pool)
                .await?;
                println!("EUR/RUB {requested_on}: rate dated {}", rate.date);
            }
            Err(error) => {
                eprintln!("EUR/RUB {requested_on}: {error:#}");
                failures += 1;
            }
        }
    }
    if failures > 0 {
        bail!("{failures} exchange-rate dates could not be imported");
    }
    Ok(())
}

async fn fetch_rate(client: &Client, requested_on: NaiveDate) -> Result<Rate> {
    let response = client
        .get("https://api.frankfurter.dev/v2/rate/EUR/RUB")
        .query(&[("date", requested_on.to_string())])
        .send()
        .await
        .context("Frankfurter request failed")?
        .error_for_status()
        .context("Frankfurter returned an error status")?;
    let rate: Rate = response
        .json()
        .await
        .context("invalid Frankfurter response")?;
    if rate.base != "EUR"
        || rate.quote != "RUB"
        || rate.date > requested_on
        || !rate.rate.is_finite()
        || rate.rate <= 0.0
    {
        bail!("invalid EUR/RUB rate response");
    }
    Ok(rate)
}
