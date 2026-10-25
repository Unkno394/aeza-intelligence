use std::path::PathBuf;
use std::time::Duration;

use aeza_intelligence::market::{Capture, parse_cdx_json, parse_html};
use aeza_intelligence::market_store::{
    ObservationSource, apply_schema, count_observations, save_offers,
};
use anyhow::{Context, Result, bail};
use chrono::{Datelike, NaiveDateTime, Utc};
use clap::{Parser, Subcommand, ValueEnum};
use reqwest::Client;
use serde::Deserialize;
use sha2::{Digest, Sha256};
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
    /// Import a previously downloaded Wayback HTML file without requesting Wayback again.
    ImportArchiveFile {
        #[arg(long, env = "DATABASE_URL", hide_env_values = true)]
        database_url: String,
        #[arg(long, value_enum)]
        provider: Provider,
        #[arg(long)]
        file: PathBuf,
        #[arg(long)]
        timestamp: String,
        #[arg(long)]
        source_url: String,
    },
    /// Import the signed Aéza public tariff snapshot used by the simulator.
    AezaSnapshot {
        #[arg(long, env = "DATABASE_URL", hide_env_values = true)]
        database_url: Option<String>,
        #[arg(long, default_value = "market-simulator/data/market-snapshot.json")]
        file: PathBuf,
        #[arg(long)]
        dry_run: bool,
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
    Contabo,
}

impl Provider {
    fn code(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Firstvds => "firstvds",
            Self::TimewebCloud => "timeweb_cloud",
            Self::Selectel => "selectel",
            Self::Hetzner => "hetzner",
            Self::Contabo => "contabo",
        }
    }

    fn selected(self) -> &'static [Provider] {
        const ALL: &[Provider] = &[
            Provider::Firstvds,
            Provider::TimewebCloud,
            Provider::Selectel,
            Provider::Hetzner,
            Provider::Contabo,
        ];
        match self {
            Self::All => ALL,
            Self::Firstvds => &ALL[0..1],
            Self::TimewebCloud => &ALL[1..2],
            Self::Selectel => &ALL[2..3],
            Self::Hetzner => &ALL[3..4],
            Self::Contabo => &ALL[4..5],
        }
    }

    fn parser_version(self) -> &'static str {
        match self {
            Self::Firstvds => "firstvds-docs-v1",
            Self::TimewebCloud => "timeweb-v1",
            Self::Selectel => "selectel-v1",
            Self::Hetzner => "hetzner-v1",
            Self::Contabo => "contabo-v1",
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

#[derive(Deserialize)]
struct SnapshotEnvelope {
    payload: String,
    sha256: String,
}

#[derive(Deserialize)]
struct AezaSnapshot {
    prepared_as_of: chrono::DateTime<Utc>,
    aeza_plans: Vec<AezaPlan>,
}

#[derive(Deserialize)]
struct AezaPlan {
    id: String,
    name: String,
    vcpu: f64,
    ram_gb: f64,
    disk_gb: f64,
    regions: Vec<String>,
    source: String,
    details: AezaDetails,
}

#[derive(Deserialize)]
struct AezaDetails {
    original_price: SnapshotPrice,
    source_region: String,
    availability: String,
    notes: Vec<String>,
}

#[derive(Deserialize)]
struct SnapshotPrice {
    amount: String,
    currency: String,
    unit: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::ImportArchiveFile {
            database_url,
            provider,
            file,
            timestamp,
            source_url,
        } => {
            if matches!(provider, Provider::All) {
                bail!("--provider must name one provider");
            }
            let observed_at = NaiveDateTime::parse_from_str(&timestamp, "%Y%m%d%H%M%S")?.and_utc();
            let html = std::fs::read_to_string(&file)
                .with_context(|| format!("cannot read {}", file.display()))?;
            let offers = parse_html(provider.code(), &html);
            let complete = offers.iter().filter(|offer| offer.is_complete()).count();
            if complete == 0 {
                bail!(
                    "{} has no complete {} tariffs",
                    file.display(),
                    provider.code()
                );
            }
            let pool = PgPoolOptions::new()
                .max_connections(4)
                .connect(&database_url)
                .await?;
            apply_schema(&pool).await?;
            let source = ObservationSource {
                kind: "archive",
                observed_at,
                source_url: source_url.clone(),
                archive_url: Some(format!(
                    "https://web.archive.org/web/{timestamp}id_/{source_url}"
                )),
                cdx_digest: None,
                parser_version: provider.parser_version(),
            };
            let saved = save_offers(&pool, provider.code(), &source, &offers).await?;
            println!(
                "{} {}: saved {saved} tariffs ({complete} complete)",
                provider.code(),
                timestamp
            );
        }
        Command::AezaSnapshot {
            database_url,
            file,
            dry_run,
        } => {
            let bytes =
                std::fs::read(&file).with_context(|| format!("cannot read {}", file.display()))?;
            let snapshot = parse_snapshot_envelope(&bytes)?;
            if dry_run {
                let available = snapshot
                    .aeza_plans
                    .iter()
                    .filter(|plan| plan.availability() == "available_from_source")
                    .count();
                println!(
                    "Aéza signed catalog: {} plans, {available} available; SHA-256 verified",
                    snapshot.aeza_plans.len()
                );
                return Ok(());
            }
            let database_url =
                database_url.context("DATABASE_URL is required unless --dry-run is used")?;
            let pool = PgPoolOptions::new()
                .max_connections(4)
                .connect(&database_url)
                .await?;
            apply_schema(&pool).await?;
            let mut total = 0;
            for plan in snapshot.aeza_plans {
                let Some(offer) = aeza_offer(&plan) else {
                    continue;
                };
                let source = ObservationSource {
                    kind: "live",
                    observed_at: snapshot.prepared_as_of,
                    source_url: plan.source,
                    archive_url: None,
                    cdx_digest: None,
                    parser_version: "aeza-signed-catalog-v1",
                };
                save_offers(&pool, "aeza", &source, &[offer]).await?;
                total += 1;
            }
            println!("Aéza signed catalog: saved {total} tariff observations");
        }
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
                .timeout(Duration::from_secs(90))
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
                            Ok((captures, cdx_failures)) => {
                                failures += cdx_failures;
                                println!(
                                    "{code} archive {url}: {} yearly captures",
                                    captures.len()
                                );
                                let capture_years: std::collections::HashSet<_> = captures
                                    .iter()
                                    .map(|capture| capture.observed_at.year())
                                    .collect();
                                let mut imported_years = std::collections::HashSet::new();
                                for capture in captures {
                                    if imported_years.contains(&capture.observed_at.year()) {
                                        continue;
                                    }
                                    tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                                    match import_capture(&client, &pool, selected, &capture).await {
                                        Ok(count) => {
                                            imported_years.insert(capture.observed_at.year());
                                            println!(
                                                "{code} {}: saved {count} tariffs",
                                                capture.timestamp
                                            );
                                        }
                                        Err(error) => {
                                            eprintln!("{code} {}: {error:#}", capture.timestamp);
                                        }
                                    }
                                }
                                failures += capture_years.difference(&imported_years).count();
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

impl AezaPlan {
    fn availability(&self) -> &str {
        &self.details.availability
    }
}

fn parse_snapshot_envelope(bytes: &[u8]) -> Result<AezaSnapshot> {
    let envelope: SnapshotEnvelope = serde_json::from_slice(bytes)?;
    let actual_sha = format!("{:x}", Sha256::digest(envelope.payload.as_bytes()));
    if actual_sha != envelope.sha256.to_ascii_lowercase() {
        bail!("Aéza catalog SHA-256 mismatch");
    }
    Ok(serde_json::from_str(&envelope.payload)?)
}

fn aeza_offer(plan: &AezaPlan) -> Option<aeza_intelligence::market::Offer> {
    if plan.availability() != "available_from_source"
        || plan.details.original_price.unit != "month"
        || !matches!(
            plan.details.original_price.currency.as_str(),
            "EUR" | "USD" | "RUB"
        )
    {
        return None;
    }
    let price_minor = decimal_minor(&plan.details.original_price.amount)?;
    let cpu_cores = positive_whole(plan.vcpu)?;
    let ram_mib = positive_whole(plan.ram_gb * 1024.0)?;
    let disk_gib = positive_whole(plan.disk_gb)?;
    Some(aeza_intelligence::market::Offer {
        plan_key: plan.name.clone(),
        plan_name: plan.name.clone(),
        cpu_cores: Some(cpu_cores),
        ram_mib: Some(ram_mib),
        disk_gib: Some(disk_gib),
        disk_type: None,
        region: plan.regions.first().cloned(),
        price_minor: Some(price_minor),
        currency: Some(plan.details.original_price.currency.clone()),
        billing_period: Some("month".to_owned()),
        price_kind: "official_catalog".to_owned(),
        vat_included: None,
        raw_excerpt: format!(
            "catalog_id: {}; source_region: {}; {}",
            plan.id,
            plan.details.source_region,
            plan.details.notes.join("; ")
        ),
    })
}

fn positive_whole(value: f64) -> Option<i32> {
    (value.is_finite() && value > 0.0 && value <= i32::MAX as f64 && value.fract() == 0.0)
        .then_some(value as i32)
}

fn decimal_minor(value: &str) -> Option<i64> {
    let (whole, fraction) = value.split_once('.').unwrap_or((value, ""));
    if !whole.chars().all(|c| c.is_ascii_digit())
        || fraction.len() > 2
        || !fraction.chars().all(|c| c.is_ascii_digit())
    {
        return None;
    }
    let whole: i64 = whole.parse().ok()?;
    let fraction: i64 = format!("{fraction:0<2}").parse().ok()?;
    whole.checked_mul(100)?.checked_add(fraction)
}

async fn list_captures(
    client: &Client,
    url: &str,
    from_year: i32,
    max_archives: usize,
) -> Result<(Vec<Capture>, usize)> {
    let mut captures = Vec::new();
    let mut failures = 0;
    for year in from_year..=Utc::now().year() {
        let params = [
            ("url", url.to_owned()),
            ("output", "json".to_owned()),
            (
                "fl",
                "timestamp,original,mimetype,statuscode,digest".to_owned(),
            ),
            ("filter", "statuscode:200".to_owned()),
            ("filter", "mimetype:text/html".to_owned()),
            ("from", year.to_string()),
            ("to", year.to_string()),
            ("collapse", "timestamp:6".to_owned()),
            ("limit", max_archives.to_string()),
        ];
        let mut result = None;
        for attempt in 0..3 {
            let response = client.get(CDX_URL).query(&params).send().await;
            match response {
                Ok(response) => match response.error_for_status() {
                    Ok(response) => match response.text().await {
                        Ok(body) => {
                            result = Some(parse_cdx_json(&body).map_err(anyhow::Error::msg)?);
                            break;
                        }
                        Err(error) => {
                            eprintln!("CDX {url} {year} attempt {}: {error}", attempt + 1)
                        }
                    },
                    Err(error) => eprintln!("CDX {url} {year} attempt {}: {error}", attempt + 1),
                },
                Err(error) => eprintln!("CDX {url} {year} attempt {}: {error}", attempt + 1),
            }
            tokio::time::sleep(Duration::from_secs(2_u64.pow(attempt))).await;
        }
        match result {
            Some(mut yearly) => captures.append(&mut yearly),
            None => failures += 1,
        }
    }
    Ok((captures, failures))
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
        if let Ok(directory) = std::env::var("ARCHIVE_DIAGNOSTICS_DIR") {
            let directory = PathBuf::from(directory);
            std::fs::create_dir_all(&directory)?;
            let path = directory.join(format!("{}-{}.html", provider.code(), capture.timestamp));
            std::fs::write(&path, &html)?;
            bail!(
                "archived page contains no complete tariffs ({} partial offers); saved HTML to {}",
                offers.len(),
                path.display()
            );
        }
        bail!(
            "archived page contains no complete tariffs ({} partial offers)",
            offers.len()
        );
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

#[cfg(test)]
mod tests {
    use super::{AezaPlan, aeza_offer, decimal_minor, parse_snapshot_envelope, positive_whole};
    use serde_json::json;
    use sha2::{Digest, Sha256};

    fn plan(
        availability: &str,
        currency: &str,
        unit: &str,
        price: &str,
        vcpu: f64,
        ram_gb: f64,
        disk_gb: f64,
    ) -> AezaPlan {
        serde_json::from_value(json!({
            "id":"basic-1", "name":"Basic", "vcpu":vcpu,
            "ram_gb":ram_gb, "disk_gb":disk_gb, "regions":["DE-FRA"],
            "source":"https://aeza.test/plans/basic",
            "details": {
                "original_price":{"amount":price,"currency":currency,"unit":unit},
                "source_region":"Frankfurt", "availability":availability, "notes":["monthly price"]
            }
        }))
        .unwrap()
    }

    #[test]
    fn signed_catalog_hash_and_payload_are_checked_before_import() {
        let payload = r#"{"prepared_as_of":"2026-10-24T00:00:00Z","aeza_plans":[]}"#;
        let digest = format!("{:x}", Sha256::digest(payload.as_bytes()));
        let envelope =
            serde_json::to_vec(&json!({"payload":payload,"sha256":digest.to_uppercase()})).unwrap();
        assert!(
            parse_snapshot_envelope(&envelope)
                .unwrap()
                .aeza_plans
                .is_empty()
        );

        let wrong_hash = serde_json::to_vec(&json!({"payload":payload,"sha256":"00"})).unwrap();
        let error = parse_snapshot_envelope(&wrong_hash).err().unwrap();
        assert!(error.to_string().contains("SHA-256 mismatch"));

        let invalid_payload = "not-json";
        let digest = format!("{:x}", Sha256::digest(invalid_payload.as_bytes()));
        let malformed =
            serde_json::to_vec(&json!({"payload":invalid_payload,"sha256":digest})).unwrap();
        assert!(parse_snapshot_envelope(&malformed).is_err());
    }

    #[test]
    fn aeza_catalog_maps_minor_currency_specs_and_region_without_rounding_unknowns() {
        let offer = aeza_offer(&plan(
            "available_from_source",
            "EUR",
            "month",
            "12.34",
            4.0,
            8.0,
            80.0,
        ))
        .unwrap();
        assert!(offer.is_complete());
        assert_eq!(offer.price_minor, Some(1234));
        assert_eq!(offer.currency.as_deref(), Some("EUR"));
        assert_eq!(offer.cpu_cores, Some(4));
        assert_eq!(offer.ram_mib, Some(8192));
        assert_eq!(offer.disk_gib, Some(80));
        assert_eq!(offer.region.as_deref(), Some("DE-FRA"));
        assert_eq!(offer.billing_period.as_deref(), Some("month"));

        for invalid in [
            plan("unknown", "EUR", "month", "12.34", 4.0, 8.0, 80.0),
            plan(
                "available_from_source",
                "EUR",
                "year",
                "12.34",
                4.0,
                8.0,
                80.0,
            ),
            plan(
                "available_from_source",
                "GBP",
                "month",
                "12.34",
                4.0,
                8.0,
                80.0,
            ),
            plan(
                "available_from_source",
                "EUR",
                "month",
                "12.345",
                4.0,
                8.0,
                80.0,
            ),
            plan(
                "available_from_source",
                "EUR",
                "month",
                "12.34",
                4.5,
                8.0,
                80.0,
            ),
            plan(
                "available_from_source",
                "EUR",
                "month",
                "12.34",
                4.0,
                0.0005,
                80.0,
            ),
            plan(
                "available_from_source",
                "EUR",
                "month",
                "12.34",
                4.0,
                8.0,
                0.0,
            ),
        ] {
            assert!(
                aeza_offer(&invalid).is_none(),
                "invalid catalog row was accepted"
            );
        }
    }

    #[test]
    fn euro_minor_amount_and_whole_resource_parsers_reject_invalid_ranges() {
        for (raw, expected) in [
            ("0", Some(0)),
            ("1", Some(100)),
            ("1.2", Some(120)),
            ("12.34", Some(1234)),
            ("12.345", None),
            ("-1.00", None),
            ("1e3", None),
        ] {
            assert_eq!(decimal_minor(raw), expected, "{raw}");
        }
        assert_eq!(positive_whole(1.0), Some(1));
        assert_eq!(positive_whole(0.0), None);
        assert_eq!(positive_whole(-1.0), None);
        assert_eq!(positive_whole(f64::NAN), None);
        assert_eq!(positive_whole(f64::INFINITY), None);
        assert_eq!(positive_whole(f64::MAX), None);
    }
}
