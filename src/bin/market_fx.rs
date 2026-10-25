use std::time::Duration;

use anyhow::{Context, Result, bail};
use chrono::{NaiveDate, Utc};
use clap::Parser;
use reqwest::Client;
use serde::Deserialize;
use sqlx::postgres::PgPoolOptions;

#[derive(Parser)]
#[command(about = "Fetch historical EUR/RUB and USD/RUB rates for imported competitor prices")]
struct Cli {
    #[arg(long, env = "DATABASE_URL", hide_env_values = true)]
    database_url: Option<String>,
    /// Fetch and parse one rate without using PostgreSQL.
    #[arg(long)]
    dry_run: bool,
    /// Date to inspect in dry-run mode; defaults to today.
    #[arg(long)]
    date: Option<NaiveDate>,
    /// Base currency for dry-run (EUR or USD).
    #[arg(long, default_value = "EUR")]
    base: String,
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
        let rate = fetch_rate(&client, &cli.base.to_uppercase(), requested_on).await?;
        println!(
            "requested_on={requested_on} rate_on={} {}/RUB={}",
            rate.date, rate.base, rate.rate
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
    aeza_intelligence::PgAdapter::from_pool(pool.clone())
        .migrate()
        .await?;
    let dates: Vec<(String, NaiveDate)> = sqlx::query_as(
        "SELECT currencies.base, observed.requested_on \
         FROM (SELECT DISTINCT (p.observed_at AT TIME ZONE 'UTC')::date AS requested_on \
               FROM market_intelligence.tariff_prices p \
               WHERE p.currency IN ('EUR', 'USD') AND p.price_minor IS NOT NULL) observed \
         CROSS JOIN (VALUES (CAST('EUR' AS text)), ('USD')) AS currencies(base) \
         WHERE NOT EXISTS (SELECT 1 FROM market_intelligence.exchange_rates fx \
           WHERE fx.base = currencies.base AND fx.quote = 'RUB' AND fx.requested_on = observed.requested_on) \
         ORDER BY 1, 2",
    )
    .fetch_all(&pool)
    .await?;
    let mut failures = 0;
    for (base, requested_on) in dates {
        match fetch_rate(&client, &base, requested_on).await {
            Ok(rate) => {
                sqlx::query(
                    "INSERT INTO market_intelligence.exchange_rates \
                     (base, quote, requested_on, rate_on, rate) VALUES ($1, 'RUB', $2, $3, $4) \
                     ON CONFLICT (base, quote, requested_on) DO UPDATE SET \
                     rate_on = EXCLUDED.rate_on, rate = EXCLUDED.rate, fetched_at = now()",
                )
                .bind(&base)
                .bind(requested_on)
                .bind(rate.date)
                .bind(rate.rate)
                .execute(&pool)
                .await?;
                println!("{base}/RUB {requested_on}: rate dated {}", rate.date);
            }
            Err(error) => {
                eprintln!("{base}/RUB {requested_on}: {error:#}");
                failures += 1;
            }
        }
    }
    if failures > 0 {
        bail!("{failures} exchange-rate dates could not be imported");
    }
    Ok(())
}

async fn fetch_rate(client: &Client, base: &str, requested_on: NaiveDate) -> Result<Rate> {
    if !matches!(base, "EUR" | "USD") {
        bail!("unsupported base currency: {base}");
    }
    let url = format!("https://api.frankfurter.dev/v2/rate/{base}/RUB");
    let response = client
        .get(url)
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
    validate_rate(base, requested_on, rate)
}

fn validate_rate(base: &str, requested_on: NaiveDate, rate: Rate) -> Result<Rate> {
    if rate.base != base
        || rate.quote != "RUB"
        || rate.date > requested_on
        || !rate.rate.is_finite()
        || rate.rate <= 0.0
    {
        bail!("invalid {base}/RUB rate response");
    }
    Ok(rate)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, day).unwrap()
    }

    fn rate(base: &str, quote: &str, day: u32, value: f64) -> Rate {
        Rate {
            date: date(day),
            base: base.into(),
            quote: quote.into(),
            rate: value,
        }
    }

    #[test]
    fn accepts_a_positive_rate_dated_on_or_before_the_observation() {
        assert!(validate_rate("EUR", date(25), rate("EUR", "RUB", 25, 93.4)).is_ok());
        assert!(validate_rate("USD", date(25), rate("USD", "RUB", 24, 81.2)).is_ok());
    }

    #[test]
    fn rejects_mismatched_currencies_future_dates_and_invalid_values() {
        for (base, value) in [
            ("EUR", rate("USD", "RUB", 25, 90.0)),
            ("EUR", rate("EUR", "USD", 25, 90.0)),
            ("EUR", rate("EUR", "RUB", 26, 90.0)),
            ("EUR", rate("EUR", "RUB", 25, 0.0)),
            ("EUR", rate("EUR", "RUB", 25, -1.0)),
            ("EUR", rate("EUR", "RUB", 25, f64::INFINITY)),
        ] {
            assert!(
                validate_rate(base, date(25), value).is_err(),
                "invalid {base}/RUB response should be rejected"
            );
        }
    }
}
