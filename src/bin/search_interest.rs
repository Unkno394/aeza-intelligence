use std::{
    io::Write,
    process::{Command as ProcessCommand, Stdio},
    time::Duration,
};

use anyhow::{Context, Result, bail};
use chrono::{NaiveDate, Utc};
use clap::{Parser, Subcommand, ValueEnum};
use reqwest::Client;
use serde::Deserialize;
use sqlx::postgres::PgPoolOptions;
use sqlx::{PgPool, Row};

const SOURCE: &str = "google search";
const API_URL: &str = "https://api.trendsmcp.ai/api";
const SCHEMA: &str = include_str!("../../migrations/002_search_interest.sql");

#[derive(Parser)]
#[command(about = "Collect weekly Google Search interest for VPS providers")]
struct Cli {
    #[arg(long, env = "DATABASE_URL", hide_env_values = true)]
    database_url: Option<String>,
    #[arg(long, env = "TRENDSMCP_API_KEY", hide_env_values = true)]
    api_key: String,
    /// HTTP transport. curl reproduces the verified terminal request.
    #[arg(long, value_enum, default_value_t = Transport::Curl)]
    transport: Transport,
    #[command(subcommand)]
    command: Command,
}

#[derive(Clone, Copy, ValueEnum)]
enum Transport {
    Curl,
    Reqwest,
}

#[derive(Subcommand)]
enum Command {
    /// Fetch brands whose last successful refresh was at least seven days ago.
    Once,
    /// Run the scheduler continuously; checks for due brands every hour.
    Schedule,
    /// Fetch and parse one keyword without using PostgreSQL.
    DryRun {
        #[arg(long, default_value = "Hetzner")]
        keyword: String,
    },
}

#[derive(Deserialize)]
struct Point {
    date: NaiveDate,
    value: f64,
    volume: Option<i64>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    if cli.api_key.trim().is_empty() {
        bail!("TRENDSMCP_API_KEY is empty");
    }
    let client = Client::builder().timeout(Duration::from_secs(45)).build()?;
    if let Command::DryRun { keyword } = &cli.command {
        let points = fetch_series(&client, &cli.api_key, keyword, cli.transport).await?;
        println!("keyword={keyword} points={}", points.len());
        for point in points.iter().take(3).chain(points.iter().rev().take(3)) {
            println!(
                "date={} value={} volume={:?}",
                point.date, point.value, point.volume
            );
        }
        return Ok(());
    }
    let database_url = cli
        .database_url
        .context("DATABASE_URL is required unless dry-run is used")?;
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect(&database_url)
        .await?;
    sqlx::raw_sql(SCHEMA).execute(&pool).await?;
    match cli.command {
        Command::Once => refresh(&pool, &client, &cli.api_key, cli.transport).await,
        Command::Schedule => loop {
            if let Err(error) = refresh(&pool, &client, &cli.api_key, cli.transport).await {
                eprintln!("search interest refresh failed: {error:#}");
            }
            tokio::time::sleep(Duration::from_secs(3600)).await;
        },
        Command::DryRun { .. } => unreachable!(),
    }
}

async fn refresh(pool: &PgPool, client: &Client, key: &str, transport: Transport) -> Result<()> {
    let brands = sqlx::query(
        "SELECT b.code, b.keyword FROM market_intelligence.search_brands b \
         WHERE b.enabled AND NOT EXISTS (SELECT 1 FROM market_intelligence.search_interest i \
         WHERE i.brand_code = b.code AND i.source = $1 AND i.collected_at > now() - interval '7 days') \
         ORDER BY b.code",
    )
    .bind(SOURCE)
    .fetch_all(pool)
    .await?;
    let mut failures = Vec::new();
    for brand in brands {
        let code: String = brand.get("code");
        let keyword: String = brand.get("keyword");
        match fetch_and_save(pool, client, key, &code, &keyword, transport).await {
            Ok(count) => println!("{code}: refreshed {count} points"),
            Err(error) => {
                eprintln!("{code}: {error:#}");
                failures.push(code);
            }
        }
    }
    if !failures.is_empty() {
        bail!("failed brands: {}", failures.join(", "));
    }
    Ok(())
}

async fn fetch_and_save(
    pool: &PgPool,
    client: &Client,
    key: &str,
    code: &str,
    keyword: &str,
    transport: Transport,
) -> Result<usize> {
    let points = fetch_series(client, key, keyword, transport).await?;
    let today = Utc::now().date_naive();
    let cutoff = today - chrono::Duration::days(90);
    let mut transaction = pool.begin().await?;
    let mut count = 0;
    for point in points {
        if point.date < cutoff || point.date > today {
            continue;
        }
        sqlx::query(
            "INSERT INTO market_intelligence.search_interest \
             (brand_code, source, observed_on, score, volume, collected_at) \
             VALUES ($1, $2, $3, $4, $5, now()) \
             ON CONFLICT (brand_code, source, observed_on) DO UPDATE SET \
             score = EXCLUDED.score, volume = EXCLUDED.volume, collected_at = EXCLUDED.collected_at",
        )
        .bind(code).bind(SOURCE).bind(point.date).bind(point.value).bind(point.volume)
        .execute(&mut *transaction).await?;
        count += 1;
    }
    if count == 0 {
        bail!("no points within the last 90 days for {code}");
    }
    transaction.commit().await?;
    Ok(count)
}

async fn fetch_series(
    client: &Client,
    key: &str,
    keyword: &str,
    transport: Transport,
) -> Result<Vec<Point>> {
    let payload =
        serde_json::json!({"mode": "get_time_series", "source": SOURCE, "keyword": keyword})
            .to_string();
    let body = match transport {
        Transport::Curl => curl_request(key, &payload)?,
        Transport::Reqwest => reqwest_request(client, key, &payload).await?,
    };
    let points = parse_points(&body).context("invalid TrendsMCP time series")?;
    if points.is_empty() {
        bail!("TrendsMCP returned an empty series");
    }
    for point in &points {
        if !point.value.is_finite()
            || !(0.0..=100.0).contains(&point.value)
            || point.volume.is_some_and(|v| v < 0)
        {
            bail!("invalid point for {keyword} on {}", point.date);
        }
    }
    Ok(points)
}

async fn reqwest_request(client: &Client, key: &str, payload: &str) -> Result<String> {
    let response = client
        .post(API_URL)
        .header(reqwest::header::ACCEPT, "*/*")
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .header(reqwest::header::USER_AGENT, "curl/8.0.1")
        .bearer_auth(key)
        .body(payload.to_owned())
        .send()
        .await
        .context("TrendsMCP request failed")?;
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    if !status.is_success() {
        bail!(
            "TrendsMCP returned {status}: {}",
            body.replace(key, "<redacted>")
                .chars()
                .take(1000)
                .collect::<String>()
        );
    }
    Ok(body)
}

fn curl_request(key: &str, payload: &str) -> Result<String> {
    if !key
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
    {
        bail!("TRENDSMCP_API_KEY has invalid characters");
    }
    let escape = |value: &str| value.replace('\\', "\\\\").replace('"', "\\\"");
    let config = format!(
        "url = \"{API_URL}\"\nrequest = \"POST\"\nheader = \"Authorization: Bearer {key}\"\nheader = \"Content-Type: application/json\"\nheader = \"Accept: */*\"\ndata = \"{}\"\nmax-time = 45\n",
        escape(payload)
    );
    let mut child = ProcessCommand::new("curl")
        .args([
            "--config",
            "-",
            "--silent",
            "--show-error",
            "--write-out",
            "\n__HTTP_STATUS__:%{http_code}",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("cannot start curl")?;
    child
        .stdin
        .take()
        .context("curl stdin unavailable")?
        .write_all(config.as_bytes())?;
    let output = child.wait_with_output()?;
    let stdout = String::from_utf8(output.stdout).context("curl response is not UTF-8")?;
    let (body, status_text) = stdout
        .rsplit_once("\n__HTTP_STATUS__:")
        .context("curl returned no HTTP status")?;
    let status: u16 = status_text
        .trim()
        .parse()
        .context("invalid curl HTTP status")?;
    if !output.status.success() || !(200..300).contains(&status) {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!(
            "TrendsMCP curl returned HTTP {status}: {} {}",
            body.replace(key, "<redacted>")
                .chars()
                .take(1000)
                .collect::<String>(),
            stderr.replace(key, "<redacted>")
        );
    }
    Ok(body.to_owned())
}

fn parse_points(body: &str) -> Result<Vec<Point>> {
    let value: serde_json::Value = serde_json::from_str(body)?;
    let rows = unwrap_points(value, 0)?;
    let mut points = Vec::with_capacity(rows.len());
    for row in rows {
        let date_value = row
            .get("date")
            .or_else(|| row.get("time"))
            .or_else(|| row.get("week"))
            .context("point has no date")?;
        let date = if let Some(text) = date_value.as_str() {
            NaiveDate::parse_from_str(text.get(..10).unwrap_or(text), "%Y-%m-%d")?
        } else if let Some(timestamp) = date_value.as_i64() {
            chrono::DateTime::<Utc>::from_timestamp(
                if timestamp > 10_000_000_000 {
                    timestamp / 1000
                } else {
                    timestamp
                },
                0,
            )
            .context("invalid point timestamp")?
            .date_naive()
        } else {
            bail!("invalid point date");
        };
        let score_value = row
            .get("value")
            .or_else(|| row.get("score"))
            .or_else(|| row.get("interest"))
            .context("point has no score")?;
        let value = score_value
            .as_f64()
            .or_else(|| score_value.as_str().and_then(|text| text.parse().ok()))
            .context("invalid point score")?;
        let volume = row.get("volume").and_then(|v| {
            v.as_i64()
                .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
        });
        points.push(Point {
            date,
            value,
            volume,
        });
    }
    points.sort_by_key(|point| point.date);
    Ok(points)
}

fn unwrap_points(value: serde_json::Value, depth: usize) -> Result<Vec<serde_json::Value>> {
    if depth > 5 {
        bail!("TrendsMCP response nested too deeply");
    }
    match value {
        serde_json::Value::Array(rows) => Ok(rows),
        serde_json::Value::String(text) => unwrap_points(serde_json::from_str(&text)?, depth + 1),
        serde_json::Value::Object(mut object) => {
            for key in ["body", "data", "results", "series", "time_series", "points"] {
                if let Some(inner) = object.remove(key) {
                    return unwrap_points(inner, depth + 1);
                }
            }
            bail!("TrendsMCP response has no time series")
        }
        _ => bail!("TrendsMCP response is not an array or body object"),
    }
}

#[cfg(test)]
mod tests {
    use super::parse_points;

    #[test]
    fn parses_wrapped_json_string_body() {
        let response = r#"{"statusCode":200,"body":"[{\"date\":\"2026-07-20\",\"value\":42,\"volume\":1200},{\"date\":\"2026-07-27\",\"value\":51,\"volume\":null}]"}"#;
        let points = parse_points(response).unwrap();
        assert_eq!(points.len(), 2);
        assert_eq!(points[0].value, 42.0);
        assert_eq!(points[1].volume, None);
    }
}
