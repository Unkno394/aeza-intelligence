//! Import a joint Google Trends Explore comparison. Explore is a public web
//! endpoint, not the restricted official Google Trends API alpha.

use std::{path::PathBuf, time::Duration};

use anyhow::{Context, Result, bail};
use chrono::{DateTime, NaiveDate, Utc};
use clap::Parser;
use reqwest::{Client, header};
use serde_json::{Value, json};
use sqlx::postgres::PgPoolOptions;

const BASE: &str = "https://trends.google.com/trends";
const SOURCE: &str = "google trends compare 5y";
const BRANDS: &[(&str, &str)] = &[
    ("aeza", "Aeza"),
    ("hetzner", "Hetzner"),
    ("contabo", "Contabo"),
    ("timeweb_cloud", "Timeweb Cloud"),
    ("selectel", "Selectel"),
];

#[derive(Parser)]
#[command(about = "Import comparable 5-year Google Trends Explore data for five VPS brands")]
struct Cli {
    #[arg(long, env = "DATABASE_URL", hide_env_values = true)]
    database_url: Option<String>,
    /// Fetch and validate data without PostgreSQL.
    #[arg(long)]
    dry_run: bool,
    /// Import Google Trends' Interest over time CSV instead of calling Explore.
    #[arg(long)]
    timeline_csv: Option<PathBuf>,
    /// Optional Interest by region CSV exported from the same comparison.
    #[arg(long, requires = "timeline_csv")]
    geo_csv: Option<PathBuf>,
}

#[derive(Debug)]
struct TimePoint {
    date: NaiveDate,
    scores: Vec<f64>,
}

#[derive(Debug)]
struct RegionPoint {
    code: String,
    name: String,
    scores: Vec<f64>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let (points, regions) = if let Some(path) = &cli.timeline_csv {
        let points = read_timeline_csv(path)?;
        let regions = match &cli.geo_csv {
            Some(path) => read_geo_csv(path)?,
            None => Vec::new(),
        };
        (points, regions)
    } else {
        load_explore().await?
    };
    if points.len() < 2 {
        bail!("Google Trends returned fewer than two complete timeline points");
    }
    let first = points.first().unwrap().date;
    let last = points.last().unwrap().date;
    println!(
        "Google Trends: {} weeks ({first} to {last}), {} countries, five brands on one scale",
        points.len(),
        regions.len()
    );
    for (index, (_, keyword)) in BRANDS.iter().enumerate() {
        println!(
            "{keyword}: latest score {}",
            points.last().unwrap().scores[index]
        );
    }
    if cli.dry_run {
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
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM market_intelligence.search_interest WHERE source = $1")
        .bind(SOURCE)
        .execute(&mut *tx)
        .await?;
    for point in &points {
        for (index, (code, _)) in BRANDS.iter().enumerate() {
            sqlx::query("INSERT INTO market_intelligence.search_interest (brand_code, source, comparison_group, observed_on, score) VALUES ($1, $2, $3, $4, $5)")
                .bind(*code).bind(SOURCE).bind("google-trends-joint:5y").bind(point.date).bind(point.scores[index])
                .execute(&mut *tx).await?;
        }
    }
    sqlx::query("DELETE FROM market_intelligence.search_interest_geo WHERE source = $1")
        .bind(SOURCE)
        .execute(&mut *tx)
        .await?;
    for region in &regions {
        let coordinates = country_centroid(&region.code);
        for (index, (code, _)) in BRANDS.iter().enumerate() {
            sqlx::query("INSERT INTO market_intelligence.search_interest_geo \
                (brand_code, source, region_code, region_name, latitude, longitude, period_start, period_end, score) \
                VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)")
                .bind(*code).bind(SOURCE).bind(&region.code).bind(&region.name)
                .bind(coordinates.map(|x| x.0)).bind(coordinates.map(|x| x.1))
                .bind(first).bind(last).bind(region.scores[index])
                .execute(&mut *tx).await?;
        }
    }
    tx.commit().await?;
    println!(
        "Saved {} time points and {} country records per brand",
        points.len(),
        regions.len()
    );
    Ok(())
}

async fn load_explore() -> Result<(Vec<TimePoint>, Vec<RegionPoint>)> {
    let client = Client::builder()
        .user_agent("Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36")
        .timeout(Duration::from_secs(45))
        .build()?;
    let cookie: Option<String> = None;
    let widgets = explore(&client, cookie.as_deref()).await?;
    let time_widget = widget(&widgets, "TIMESERIES")?;
    let points = fetch_timeline(&client, cookie.as_deref(), time_widget).await?;
    let regions = match widget(&widgets, "GEO_MAP") {
        Ok(geo_widget) => match fetch_regions(&client, cookie.as_deref(), geo_widget).await {
            Ok(rows) => rows,
            Err(error) => {
                eprintln!("geography unavailable: {error:#}");
                Vec::new()
            }
        },
        Err(error) => {
            eprintln!("geography unavailable: {error:#}");
            Vec::new()
        }
    };
    Ok((points, regions))
}

fn read_timeline_csv(path: &PathBuf) -> Result<Vec<TimePoint>> {
    let rows = read_export_rows(path, &["Week", "Day", "Month", "Year", "Date"])?;
    let mut points = Vec::new();
    for row in rows {
        let date_text = row[0].trim();
        let date = if date_text.len() >= 10 {
            NaiveDate::parse_from_str(&date_text[..10], "%Y-%m-%d")?
        } else if date_text.len() == 7 {
            NaiveDate::parse_from_str(&format!("{date_text}-01"), "%Y-%m-%d")?
        } else {
            NaiveDate::parse_from_str(&format!("{date_text}-01-01"), "%Y-%m-%d")?
        };
        points.push(TimePoint {
            date,
            scores: csv_scores(&row)?,
        });
    }
    points.sort_by_key(|point| point.date);
    Ok(points)
}

fn read_geo_csv(path: &PathBuf) -> Result<Vec<RegionPoint>> {
    let rows = read_export_rows(path, &["Country", "Region", "Subregion"])?;
    rows.into_iter()
        .map(|row| {
            let name = row[0].trim().to_owned();
            let code = country_code(&name).unwrap_or(&name).to_owned();
            Ok(RegionPoint {
                code,
                name,
                scores: csv_scores(&row)?,
            })
        })
        .collect()
}

fn read_export_rows(path: &PathBuf, accepted_headers: &[&str]) -> Result<Vec<Vec<String>>> {
    let content =
        std::fs::read_to_string(path).with_context(|| format!("cannot read {}", path.display()))?;
    let mut lines = content
        .lines()
        .map(|line| parse_csv_line(line.trim_start_matches('\u{feff}')));
    let header = lines
        .find(|row| {
            row.first().is_some_and(|first| {
                accepted_headers
                    .iter()
                    .any(|name| first.eq_ignore_ascii_case(name))
            })
        })
        .context("Google Trends CSV header was not found")?;
    if header.len() != BRANDS.len() + 1 {
        bail!("CSV must contain one date/region column and exactly five brand columns");
    }
    for (index, (_, keyword)) in BRANDS.iter().enumerate() {
        if !header[index + 1]
            .to_lowercase()
            .contains(&keyword.to_lowercase())
        {
            bail!("CSV column {} must be {keyword}", index + 2);
        }
    }
    let rows: Vec<Vec<String>> = lines
        .filter(|row| row.first().is_some_and(|first| !first.trim().is_empty()))
        .collect();
    if rows.is_empty() {
        bail!("Google Trends CSV has no data rows");
    }
    if rows.iter().any(|row| row.len() != header.len()) {
        bail!("inconsistent CSV column count");
    }
    Ok(rows)
}

fn parse_csv_line(line: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut current = String::new();
    let mut chars = line.chars().peekable();
    let mut quoted = false;
    while let Some(ch) = chars.next() {
        match ch {
            '"' if quoted && chars.peek() == Some(&'"') => {
                current.push('"');
                chars.next();
            }
            '"' => quoted = !quoted,
            ',' if !quoted => {
                fields.push(current.trim().to_owned());
                current.clear();
            }
            _ => current.push(ch),
        }
    }
    fields.push(current.trim().to_owned());
    fields
}

fn csv_scores(row: &[String]) -> Result<Vec<f64>> {
    row.iter()
        .skip(1)
        .map(|value| {
            let score = if value.trim() == "<1" {
                0.5
            } else {
                value.trim().parse::<f64>()?
            };
            if !(0.0..=100.0).contains(&score) {
                bail!("CSV score outside 0–100");
            }
            Ok(score)
        })
        .collect()
}

fn country_code(name: &str) -> Option<&'static str> {
    Some(match name {
        "Russia" => "RU",
        "Germany" => "DE",
        "United States" => "US",
        "France" => "FR",
        "United Kingdom" => "GB",
        "Netherlands" => "NL",
        "Poland" => "PL",
        "Ukraine" => "UA",
        "Belarus" => "BY",
        "Turkey" | "Türkiye" => "TR",
        "Kazakhstan" => "KZ",
        "Uzbekistan" => "UZ",
        "Canada" => "CA",
        "Brazil" => "BR",
        "Mexico" => "MX",
        "Argentina" => "AR",
        "Chile" => "CL",
        "China" => "CN",
        "India" => "IN",
        "Japan" => "JP",
        "South Korea" => "KR",
        "Australia" => "AU",
        "Singapore" => "SG",
        "Indonesia" => "ID",
        "Thailand" => "TH",
        "Vietnam" => "VN",
        "Malaysia" => "MY",
        "Philippines" => "PH",
        "Pakistan" => "PK",
        "Bangladesh" => "BD",
        "United Arab Emirates" => "AE",
        "Saudi Arabia" => "SA",
        "Israel" => "IL",
        "Iran" => "IR",
        "Egypt" => "EG",
        "South Africa" => "ZA",
        "Italy" => "IT",
        "Spain" => "ES",
        "Portugal" => "PT",
        "Sweden" => "SE",
        "Norway" => "NO",
        "Finland" => "FI",
        "Denmark" => "DK",
        "Switzerland" => "CH",
        "Austria" => "AT",
        "Czechia" | "Czech Republic" => "CZ",
        "Slovakia" => "SK",
        "Hungary" => "HU",
        "Romania" => "RO",
        "Bulgaria" => "BG",
        "Serbia" => "RS",
        "Greece" => "GR",
        "Lithuania" => "LT",
        "Latvia" => "LV",
        "Estonia" => "EE",
        "Georgia" => "GE",
        "Armenia" => "AM",
        "Azerbaijan" => "AZ",
        "Moldova" => "MD",
        "Ireland" => "IE",
        "Belgium" => "BE",
        "Luxembourg" => "LU",
        "New Zealand" => "NZ",
        _ => return None,
    })
}

async fn explore(client: &Client, cookie: Option<&str>) -> Result<Value> {
    let comparison: Vec<Value> = BRANDS
        .iter()
        .map(|(_, keyword)| json!({"keyword":keyword,"geo":"","time":"today 5-y"}))
        .collect();
    let request = json!({"comparisonItem":comparison,"category":0,"property":""}).to_string();
    let mut call = client
        .post(format!("{BASE}/api/explore"))
        .query(&[("hl", "en-US"), ("tz", "0"), ("req", request.as_str())])
        .header(header::CONTENT_LENGTH, "0")
        .body("");
    if let Some(cookie) = cookie {
        call = call.header(header::COOKIE, cookie);
    }
    let value = fetch_json(call).await?;
    value
        .get("widgets")
        .and_then(Value::as_array)
        .cloned()
        .map(Value::Array)
        .context("Google Trends Explore returned no widgets")
}

fn widget<'a>(widgets: &'a Value, id: &str) -> Result<&'a Value> {
    widgets
        .as_array()
        .and_then(|items| items.iter().find(|item| item["id"] == id))
        .with_context(|| format!("Google Trends returned no {id} widget"))
}

async fn widget_data(
    client: &Client,
    cookie: Option<&str>,
    widget: &Value,
    endpoint: &str,
    geo: bool,
) -> Result<Value> {
    let token = widget["token"].as_str().context("widget has no token")?;
    let mut request = widget["request"].clone();
    if geo {
        request["resolution"] = json!("COUNTRY");
        request["includeLowSearchVolumeGeos"] = json!(true);
    }
    let request = request.to_string();
    let mut call = client
        .get(format!("{BASE}/api/widgetdata/{endpoint}"))
        .query(&[("req", request.as_str()), ("token", token), ("tz", "0")]);
    if let Some(cookie) = cookie {
        call = call.header(header::COOKIE, cookie);
    }
    fetch_json(call).await
}

async fn fetch_timeline(
    client: &Client,
    cookie: Option<&str>,
    widget: &Value,
) -> Result<Vec<TimePoint>> {
    let value = widget_data(client, cookie, widget, "multiline", false).await?;
    let rows = value
        .pointer("/default/timelineData")
        .and_then(Value::as_array)
        .context("timelineData is missing")?;
    let mut points = Vec::new();
    for row in rows {
        if row["isPartial"].as_bool().unwrap_or(false) {
            continue;
        }
        let seconds = row["time"]
            .as_str()
            .context("timeline time is missing")?
            .parse::<i64>()?;
        let date = DateTime::<Utc>::from_timestamp(seconds, 0)
            .context("invalid timeline timestamp")?
            .date_naive();
        let scores = scores(row, BRANDS.len())?;
        points.push(TimePoint { date, scores });
    }
    points.sort_by_key(|point| point.date);
    Ok(points)
}

async fn fetch_regions(
    client: &Client,
    cookie: Option<&str>,
    widget: &Value,
) -> Result<Vec<RegionPoint>> {
    let value = widget_data(client, cookie, widget, "comparedgeo", true).await?;
    let rows = value
        .pointer("/default/geoMapData")
        .and_then(Value::as_array)
        .context("geoMapData is missing")?;
    let mut regions = Vec::new();
    for row in rows {
        let Some(code) = row["geoCode"].as_str() else {
            continue;
        };
        let Some(name) = row["geoName"].as_str() else {
            continue;
        };
        regions.push(RegionPoint {
            code: code.to_owned(),
            name: name.to_owned(),
            scores: scores(row, BRANDS.len())?,
        });
    }
    Ok(regions)
}

fn scores(row: &Value, count: usize) -> Result<Vec<f64>> {
    let values = row["value"].as_array().context("row has no value array")?;
    if values.len() != count {
        bail!(
            "Google Trends returned {} values; expected {count}",
            values.len()
        );
    }
    values
        .iter()
        .map(|value| {
            let score = value.as_f64().context("score is not numeric")?;
            if !(0.0..=100.0).contains(&score) {
                bail!("score outside 0–100");
            }
            Ok(score)
        })
        .collect()
}

async fn fetch_json(call: reqwest::RequestBuilder) -> Result<Value> {
    let response = call.send().await.context("Google Trends request failed")?;
    let status = response.status();
    if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
        bail!(
            "Google Trends limits automated Explore requests (HTTP 429). Export the joint comparison as CSV and run: bash scripts/collect_trends.sh --timeline-csv /path/to/multiTimeline.csv --geo-csv /path/to/geoMap.csv"
        );
    }
    let body = response.text().await?;
    if !status.is_success() {
        bail!(
            "Google Trends returned {status}: {}",
            body.chars().take(250).collect::<String>()
        );
    }
    let start = body
        .find('{')
        .context("Google Trends returned no JSON object")?;
    serde_json::from_str(&body[start..]).context("invalid Google Trends JSON")
}

fn country_centroid(code: &str) -> Option<(f64, f64)> {
    Some(match code {
        "RU" => (61.5, 105.3),
        "DE" => (51.2, 10.5),
        "US" => (39.8, -98.6),
        "FR" => (46.6, 2.2),
        "GB" => (54.0, -2.5),
        "NL" => (52.1, 5.3),
        "PL" => (52.0, 19.1),
        "UA" => (48.4, 31.2),
        "BY" => (53.7, 27.9),
        "TR" => (39.0, 35.2),
        "KZ" => (48.0, 68.0),
        "UZ" => (41.4, 64.6),
        "CA" => (56.1, -106.3),
        "BR" => (-14.2, -51.9),
        "MX" => (23.6, -102.5),
        "AR" => (-38.4, -63.6),
        "CL" => (-35.7, -71.5),
        "CN" => (35.9, 104.2),
        "IN" => (20.6, 78.9),
        "JP" => (36.2, 138.3),
        "KR" => (36.0, 127.8),
        "AU" => (-25.3, 133.8),
        "SG" => (1.35, 103.8),
        "ID" => (-0.8, 113.9),
        "TH" => (15.9, 100.9),
        "VN" => (14.1, 108.3),
        "MY" => (4.2, 101.9),
        "PH" => (12.9, 121.8),
        "PK" => (30.4, 69.3),
        "BD" => (23.7, 90.4),
        "AE" => (23.4, 53.8),
        "SA" => (23.9, 45.1),
        "IL" => (31.0, 34.9),
        "IR" => (32.4, 53.7),
        "EG" => (26.8, 30.8),
        "ZA" => (-30.6, 22.9),
        "IT" => (41.9, 12.6),
        "ES" => (40.5, -3.7),
        "PT" => (39.4, -8.2),
        "SE" => (60.1, 18.6),
        "NO" => (60.5, 8.5),
        "FI" => (61.9, 25.7),
        "DK" => (56.3, 9.5),
        "CH" => (46.8, 8.2),
        "AT" => (47.5, 14.6),
        "CZ" => (49.8, 15.5),
        "SK" => (48.7, 19.7),
        "HU" => (47.2, 19.5),
        "RO" => (45.9, 24.9),
        "BG" => (42.7, 25.5),
        "RS" => (44.0, 21.0),
        "GR" => (39.1, 21.8),
        "LT" => (55.2, 23.9),
        "LV" => (56.9, 24.6),
        "EE" => (58.6, 25.0),
        "GE" => (42.3, 43.4),
        "AM" => (40.1, 45.0),
        "AZ" => (40.1, 47.6),
        "MD" => (47.4, 28.4),
        "IE" => (53.1, -8.2),
        "BE" => (50.5, 4.5),
        "LU" => (49.8, 6.1),
        "NZ" => (-40.9, 174.9),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::{parse_csv_line, scores};
    use serde_json::json;

    #[test]
    fn rejects_incomplete_comparison_row() {
        assert!(scores(&json!({"value":[10,20]}), 5).is_err());
        assert_eq!(
            scores(&json!({"value":[0,20,50,80,100]}), 5).unwrap().len(),
            5
        );
    }

    #[test]
    fn parses_quoted_google_export_country() {
        assert_eq!(
            parse_csv_line("\"Korea, South\",1,2,3,4,5"),
            ["Korea, South", "1", "2", "3", "4", "5"]
        );
    }
}
