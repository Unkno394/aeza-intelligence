use std::path::PathBuf;
use std::time::Duration;

use aeza_intelligence::market::{Capture, parse_cdx_json, parse_html};
use aeza_intelligence::market_store::{
    ObservationSource, apply_schema, count_observations, save_offers,
};
use anyhow::{Context, Result, bail};
use chrono::{Datelike, Utc};
use clap::{Parser, Subcommand, ValueEnum};
use reqwest::Client;
use sqlx::postgres::PgPoolOptions;

const CDX_URL: &str = "https://web.archive.org/cdx/search/cdx";
const MAX_HTML_BYTES: usize = 8 * 1024 * 1024;

#[derive(Parser)]
#[command(about = "Collect competitor VPS tariffs from live pages and Wayback")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Check a downloaded HTML page without a database connection.
    Inspect {
        #[arg(long, value_enum)]
        provider: Provider,
        #[arg(long)]
        file: PathBuf,
    },
    /// Create market_intelligence tables and import verified observations.
    Run {
        #[arg(long, env = "DATABASE_URL", hide_env_values = true)]
        database_url: String,
        #[arg(long, value_enum, default_value_t = Provider::All)]
        provider: Provider,
        #[arg(long, value_enum, default_value_t = Mode::Both)]
        mode: Mode,
        #[arg(long, default_value_t = 2024)]
        from_year: i32,
        /// Maximum yearly archive captures per URL (small by default).
        #[arg(long, default_value_t = 4)]
        max_archives: usize,
        #[arg(long, default_value_t = 1500)]
        delay_ms: u64,
    },
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum Provider {
    All,
    Firstvds,
    TimewebCloud,
    Selectel,
    Hetzner,
}

impl Provider {
    fn code(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Firstvds => "firstvds",
            Self::TimewebCloud => "timeweb_cloud",
            Self::Selectel => "selectel",
            Self::Hetzner => "hetzner",
        }
    }

    fn selected(self) -> &'static [Provider] {
        const ALL: &[Provider] = &[
            Provider::Firstvds,
            Provider::TimewebCloud,
            Provider::Selectel,
            Provider::Hetzner,
        ];
        match self {
            Self::All => ALL,
            Self::Firstvds => &ALL[0..1],
            Self::TimewebCloud => &ALL[1..2],
            Self::Selectel => &ALL[2..3],
            Self::Hetzner => &ALL[3..4],
        }
    }

    fn parser_version(self) -> &'static str {
        match self {
            Self::Firstvds => "firstvds-docs-v1",
            Self::TimewebCloud => "timeweb-v1",
            Self::Selectel => "selectel-v1",
            Self::Hetzner => "hetzner-v1",
            Self::All => "none",
        }
    }
}

#[derive(Clone, Copy, Debug, ValueEnum, PartialEq, Eq)]
enum Mode {
    Live,
    Archive,
    Both,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Inspect { provider, file } => {
            let html = std::fs::read_to_string(&file)
                .with_context(|| format!("cannot read {}", file.display()))?;
            let offers = parse_html(provider.code(), &html);
            let complete = offers.iter().filter(|offer| offer.is_complete()).count();
            println!(
                "{}: {} tariffs, {} complete",
                provider.code(),
                offers.len(),
                complete
            );
            for offer in offers.iter().take(5) {
                println!(
                    "  {} | CPU {:?} | RAM {:?} MiB | disk {:?} GiB | price {:?} {:?}/month",
                    offer.plan_key,
                    offer.cpu_cores,
                    offer.ram_mib,
                    offer.disk_gib,
                    offer.price_minor,
                    offer.currency
                );
            }
            if offers.is_empty() || complete == 0 {
                bail!("no complete tariffs found; inspect the page structure");
            }
        }
        Command::Run {
            database_url,
            provider,
            mode,
            from_year,
            max_archives,
            delay_ms,
        } => {
            let pool = PgPoolOptions::new()
                .max_connections(4)
                .connect(&database_url)
                .await
                .context("cannot connect to the existing PostgreSQL")?;
            apply_schema(&pool)
                .await
                .context("market migration failed")?;
            let client = Client::builder()
                .user_agent("AezaMarketIntelligence/0.1 (public pricing research)")
                .timeout(Duration::from_secs(45))
                .redirect(reqwest::redirect::Policy::limited(5))
                .build()?;
            let mut failures = 0usize;
            for &selected in provider.selected() {
                let code = selected.code();
                let urls: Vec<String> = sqlx::query_scalar(
                    "SELECT tariff_page_urls FROM market_intelligence.providers WHERE code = $1",
                )
                .bind(code)
                .fetch_one(&pool)
                .await?;
                for url in urls {
                    if mode != Mode::Archive {
                        match fetch_html(&client, &url).await {
                            Ok((html, final_url)) => {
                                let offers = parse_html(code, &html);
                                let complete =
                                    offers.iter().filter(|offer| offer.is_complete()).count();
                                if complete == 0 {
                                    eprintln!("{code} live {url}: no complete tariffs");
                                    failures += 1;
                                } else {
                                    let source = ObservationSource {
                                        kind: "live",
                                        observed_at: Utc::now(),
                                        source_url: final_url,
                                        archive_url: None,
                                        cdx_digest: None,
                                        parser_version: selected.parser_version(),
                                    };
                                    save_offers(&pool, code, &source, &offers).await?;
                                    println!(
                                        "{code} live: saved {} tariffs ({complete} complete)",
                                        offers.len()
                                    );
                                }
                            }
                            Err(error) => {
                                eprintln!("{code} live {url}: {error:#}");
                                failures += 1;
                            }
                        }
                    }
                    if mode != Mode::Live && max_archives > 0 {
                        match list_captures(&client, &url, from_year, max_archives).await {
                            Ok(captures) => {
                                println!(
                                    "{code} archive {url}: {} yearly captures",
                                    captures.len()
                                );
                                for capture in captures {
                                    tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                                    match import_capture(&client, &pool, selected, &capture).await {
                                        Ok(count) => println!(
                                            "{code} {}: saved {count} tariffs",
                                            capture.timestamp
                                        ),
                                        Err(error) => {
                                            eprintln!("{code} {}: {error:#}", capture.timestamp);
                                            failures += 1;
                                        }
                                    }
                                }
                            }
                            Err(error) => {
                                eprintln!("{code} CDX {url}: {error:#}");
                                failures += 1;
                            }
                        }
                    }
                }
                let persisted = count_observations(&pool, code).await?;
                println!("{code}: {persisted} price observations in PostgreSQL");
            }
            if failures > 0 {
                bail!("{failures} sources or captures could not be imported; see errors above");
            }
        }
    }
    Ok(())
}

async fn list_captures(
    client: &Client,
    url: &str,
    from_year: i32,
    max_archives: usize,
) -> Result<Vec<Capture>> {
    let params = [
        ("url", url.to_owned()),
        ("output", "json".to_owned()),
        (
            "fl",
            "timestamp,original,mimetype,statuscode,digest".to_owned(),
        ),
        ("filter", "statuscode:200".to_owned()),
        ("filter", "mimetype:text/html".to_owned()),
        ("from", from_year.to_string()),
        ("to", Utc::now().year().to_string()),
        ("collapse", "timestamp:4".to_owned()),
        ("limit", max_archives.to_string()),
    ];
    let response = client
        .get(CDX_URL)
        .query(&params)
        .send()
        .await?
        .error_for_status()?;
    let body = response.text().await?;
    parse_cdx_json(&body).map_err(anyhow::Error::msg)
}

async fn import_capture(
    client: &Client,
    pool: &sqlx::PgPool,
    provider: Provider,
    capture: &Capture,
) -> Result<usize> {
    let replay_url = capture.replay_url();
    let (html, final_url) = fetch_html(client, &replay_url).await?;
    if !final_url.contains(&capture.timestamp) {
        bail!("Wayback redirected to a different capture: {final_url}");
    }
    let offers = parse_html(provider.code(), &html);
    let complete = offers.iter().filter(|offer| offer.is_complete()).count();
    if complete == 0 {
        bail!("archived page contains no complete tariffs");
    }
    let source = ObservationSource {
        kind: "archive",
        observed_at: capture.observed_at,
        source_url: capture.original_url.clone(),
        archive_url: Some(final_url),
        cdx_digest: Some(capture.digest.clone()),
        parser_version: provider.parser_version(),
    };
    save_offers(pool, provider.code(), &source, &offers)
        .await
        .map_err(Into::into)
}

async fn fetch_html(client: &Client, url: &str) -> Result<(String, String)> {
    let response = client.get(url).send().await?.error_for_status()?;
    let final_url = response.url().to_string();
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    if !content_type.contains("text/html") {
        bail!("unexpected content type {content_type}");
    }
    let bytes = response.bytes().await?;
    if bytes.len() > MAX_HTML_BYTES {
        bail!("HTML exceeds {} bytes", MAX_HTML_BYTES);
    }
    let html = String::from_utf8(bytes.to_vec()).context("HTML is not UTF-8")?;
    Ok((html, final_url))
}
