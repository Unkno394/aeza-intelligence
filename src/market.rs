//! Small, provider-specific extractors for publicly visible VPS tariff pages.
//! An offer is only considered complete when price, CPU, RAM and disk are found.

use std::collections::HashSet;

use chrono::{DateTime, NaiveDateTime, Utc};
use scraper::{ElementRef, Html, Selector};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct Offer {
    pub plan_key: String,
    pub plan_name: String,
    pub cpu_cores: Option<i32>,
    pub ram_mib: Option<i32>,
    pub disk_gib: Option<i32>,
    pub disk_type: Option<String>,
    pub region: Option<String>,
    pub price_minor: Option<i64>,
    pub currency: Option<String>,
    pub billing_period: Option<String>,
    pub price_kind: String,
    pub vat_included: Option<bool>,
    pub raw_excerpt: String,
}

impl Offer {
    pub fn is_complete(&self) -> bool {
        self.price_minor.is_some()
            && self.currency.is_some()
            && self.billing_period.is_some()
            && self.cpu_cores.is_some()
            && self.ram_mib.is_some()
            && self.disk_gib.is_some()
    }

    pub fn configuration_fingerprint(&self) -> String {
        format!(
            "cpu={:?}|ram={:?}|disk={:?}|type={:?}|region={:?}",
            self.cpu_cores, self.ram_mib, self.disk_gib, self.disk_type, self.region
        )
    }
}

#[derive(Debug, Clone)]
pub struct Capture {
    pub timestamp: String,
    pub observed_at: DateTime<Utc>,
    pub original_url: String,
    pub digest: String,
}

impl Capture {
    pub fn replay_url(&self) -> String {
        format!(
            "https://web.archive.org/web/{}id_/{}",
            self.timestamp, self.original_url
        )
    }
}

pub fn parse_cdx_json(body: &str) -> Result<Vec<Capture>, String> {
    let rows: Vec<Vec<String>> = serde_json::from_str(body).map_err(|error| error.to_string())?;
    let Some(header) = rows.first() else {
        return Ok(Vec::new());
    };
    let column = |name: &str| {
        header
            .iter()
            .position(|value| value == name)
            .ok_or_else(|| format!("CDX response is missing {name}"))
    };
    let timestamp_col = column("timestamp")?;
    let original_col = column("original")?;
    let digest_col = column("digest")?;
    let mut captures = Vec::new();
    for row in rows.iter().skip(1) {
        let (Some(timestamp), Some(original_url), Some(digest)) = (
            row.get(timestamp_col),
            row.get(original_col),
            row.get(digest_col),
        ) else {
            continue;
        };
        if !original_url.starts_with("http://") && !original_url.starts_with("https://") {
            continue;
        }
        let observed_at = NaiveDateTime::parse_from_str(timestamp, "%Y%m%d%H%M%S")
            .map_err(|error| format!("invalid CDX timestamp {timestamp}: {error}"))?
            .and_utc();
        captures.push(Capture {
            timestamp: timestamp.clone(),
            observed_at,
            original_url: original_url.clone(),
            digest: digest.clone(),
        });
    }
    Ok(captures)
}

pub fn parse_html(provider: &str, html: &str) -> Vec<Offer> {
    let document = Html::parse_document(html);
    let offers = match provider {
        "firstvds" => parse_firstvds(&document),
        "timeweb_cloud" => {
            let current = parse_timeweb(&document);
            if current.is_empty() {
                parse_timeweb_legacy(&document)
            } else {
                current
            }
        }
        "selectel" => parse_selectel(&document),
        "hetzner" => {
            let current = parse_hetzner_current(&document);
            if current.is_empty() {
                parse_hetzner_legacy(&document)
            } else {
                current
            }
        }
        _ => Vec::new(),
    };
    let mut seen = HashSet::new();
    offers
        .into_iter()
        .filter(|offer| {
            seen.insert((
                offer.plan_key.clone(),
                offer.configuration_fingerprint(),
                offer.price_minor,
                offer.billing_period.clone(),
            ))
        })
        .collect()
}

fn parse_firstvds(document: &Html) -> Vec<Offer> {
    let table = selector("table");
    let tr = selector("tr");
    let cell = selector("th, td");
    let mut offers = Vec::new();
    for table in document.select(&table) {
        let rows: Vec<Vec<String>> = table
            .select(&tr)
            .map(|row| row.select(&cell).map(normalized_text).collect())
            .collect();
        let Some(names) = rows
            .iter()
            .find(|row| row.first().is_some_and(|v| v == "Тариф"))
        else {
            continue;
        };
        let find_row = |needle: &str| {
            rows.iter()
                .find(|row| row.first().is_some_and(|label| label.contains(needle)))
        };
        let (Some(cpu), Some(ram), Some(disk), Some(price)) = (
            find_row("Количество процессоров"),
            find_row("Оперативная память"),
            find_row("Дисковое пространство"),
            find_row("Стоимость за период: месяц"),
        ) else {
            continue;
        };
        for column in 1..names.len() {
            let name = &names[column];
            if !name.starts_with("VDS-") {
                continue;
            }
            offers.push(Offer {
                plan_key: name.clone(),
                plan_name: name.clone(),
                cpu_cores: cpu.get(column).and_then(|value| first_int(value)),
                ram_mib: ram.get(column).and_then(|value| memory_mib(value)),
                disk_gib: disk.get(column).and_then(|value| disk_gib(value)),
                disk_type: if name.to_uppercase().contains("NVME") {
                    Some("NVMe".to_owned())
                } else if name.to_uppercase().contains("SSD") {
                    Some("SSD".to_owned())
                } else {
                    None
                },
                region: None,
                price_minor: price.get(column).and_then(|value| money_minor(value)),
                currency: Some("RUB".to_owned()),
                billing_period: Some("month".to_owned()),
                price_kind: "regular".to_owned(),
                vat_included: None,
                raw_excerpt: excerpt(&format!(
                    "{name}; CPU={}; RAM={}; disk={}; price={}",
                    cpu.get(column).map_or("", String::as_str),
                    ram.get(column).map_or("", String::as_str),
                    disk.get(column).map_or("", String::as_str),
                    price.get(column).map_or("", String::as_str)
                )),
            });
        }
    }
    offers
}

fn parse_timeweb(document: &Html) -> Vec<Offer> {
    let row = selector("div[class*='row--']");
    let title = selector("div[class*='titleWrapper'] span");
    let price = selector("h5");
    let specs = selector("div[class*='wFixed']");
    let mut offers = Vec::new();
    for row in document.select(&row) {
        let Some(name) = row.select(&title).next().map(normalized_text) else {
            continue;
        };
        let Some(price_text) = row.select(&price).next().map(normalized_text) else {
            continue;
        };
        let values: Vec<_> = row.select(&specs).map(normalized_text).collect();
        if values.len() < 3 || !price_text.contains('₽') {
            continue;
        }
        offers.push(Offer {
            plan_key: name.clone(),
            plan_name: name,
            cpu_cores: first_int(&values[0]),
            ram_mib: memory_mib(&values[1]),
            disk_gib: disk_gib(&values[2]),
            disk_type: Some("NVMe".to_owned()),
            region: None,
            price_minor: money_minor(&price_text),
            currency: Some("RUB".to_owned()),
            billing_period: Some("month".to_owned()),
            price_kind: "regular".to_owned(),
            vat_included: None,
            raw_excerpt: excerpt(&format!("{price_text}; {}", values.join("; "))),
        });
    }
    offers
}

fn parse_timeweb_legacy(document: &Html) -> Vec<Offer> {
    let table = selector("table");
    let caption = selector("caption");
    let tr = selector("tr");
    let cell = selector("th, td");
    let mut offers = Vec::new();
    for table in document.select(&table) {
        if !table
            .select(&caption)
            .next()
            .is_some_and(|value| normalized_text(value).contains("Таблица тарифов"))
        {
            continue;
        }
        let rows: Vec<Vec<String>> = table
            .select(&tr)
            .map(|row| row.select(&cell).map(normalized_text).collect())
            .collect();
        let Some(header) = rows.first() else {
            continue;
        };
        let find_row = |label: &str| {
            rows.iter()
                .find(|row| row.first().is_some_and(|value| value.contains(label)))
        };
        let (Some(cpu), Some(ram), Some(disk)) = (
            find_row("Процессор"),
            find_row("Память"),
            find_row("SSD NVMe"),
        ) else {
            continue;
        };
        for column in 1..header.len() {
            let name = header[column]
                .split('—')
                .next()
                .unwrap_or("")
                .trim()
                .to_owned();
            if !name.starts_with("Cloud ") {
                continue;
            }
            offers.push(Offer {
                plan_key: name.clone(),
                plan_name: name,
                cpu_cores: cpu.get(column).and_then(|value| first_int(value)),
                ram_mib: ram.get(column).and_then(|value| memory_mib(value)),
                disk_gib: disk.get(column).and_then(|value| disk_gib(value)),
                disk_type: Some("NVMe".to_owned()),
                region: None,
                price_minor: header[column].split('—').nth(1).and_then(money_minor),
                currency: Some("RUB".to_owned()),
                billing_period: Some("month".to_owned()),
                price_kind: "regular".to_owned(),
                vat_included: None,
                raw_excerpt: excerpt(&format!(
                    "{}; CPU={}; RAM={}; disk={}",
                    header[column],
                    cpu.get(column).map_or("", String::as_str),
                    ram.get(column).map_or("", String::as_str),
                    disk.get(column).map_or("", String::as_str)
                )),
            });
        }
    }
    offers
}

fn parse_selectel(document: &Html) -> Vec<Offer> {
    let card = selector("a.custom-card");
    let title = selector("h5");
    let tr = selector("tr");
    let td = selector("td");
    let price = selector(".custom-card__price");
    let mut offers = Vec::new();
    for card in document.select(&card) {
        let Some(name) = card.select(&title).next().map(normalized_text) else {
            continue;
        };
        if !name.starts_with("VDS ") {
            continue;
        }
        let mut cpu = None;
        let mut ram = None;
        let mut disk = None;
        for row in card.select(&tr) {
            let cells: Vec<_> = row.select(&td).map(normalized_text).collect();
            if cells.len() != 2 {
                continue;
            }
            match cells[0].as_str() {
                "vCPU" => cpu = first_int(&cells[1]),
                "RAM" => ram = memory_mib(&cells[1]),
                "Диск" => disk = disk_gib(&cells[1]),
                _ => {}
            }
        }
        let price_text = card
            .select(&price)
            .next()
            .map(normalized_text)
            .unwrap_or_default();
        offers.push(Offer {
            plan_key: name.clone(),
            plan_name: name,
            cpu_cores: cpu,
            ram_mib: ram,
            disk_gib: disk,
            disk_type: Some("NVMe".to_owned()),
            region: None,
            price_minor: money_minor(&price_text),
            currency: Some("RUB".to_owned()),
            billing_period: Some("month".to_owned()),
            price_kind: "regular".to_owned(),
            vat_included: None,
            raw_excerpt: excerpt(&normalized_text(card)),
        });
    }
    offers
}

fn parse_hetzner_current(document: &Html) -> Vec<Offer> {
    let card = selector("div.cloud-matrix-product");
    let name = selector(".name-cell");
    let cpu = selector(".cpu-cell");
    let ram = selector(".ram-cell");
    let disk = selector(".drive-cell");
    let price = selector(".month-price-cell .visually-hidden");
    let price_cell = selector(".month-price-cell");
    let mut offers = Vec::new();
    for card in document.select(&card) {
        let Some(plan_name) = card.select(&name).next().map(normalized_text) else {
            continue;
        };
        if !is_hetzner_plan(&plan_name) {
            continue;
        }
        let Some(price_text) = card.select(&price).next().map(normalized_text) else {
            continue;
        };
        if !price_text.contains('€') {
            continue;
        }
        let cpu_text = card
            .select(&cpu)
            .next()
            .map(normalized_text)
            .unwrap_or_default();
        let ram_text = card
            .select(&ram)
            .next()
            .map(normalized_text)
            .unwrap_or_default();
        let disk_text = card
            .select(&disk)
            .next()
            .map(normalized_text)
            .unwrap_or_default();
        let starting_from = card
            .select(&price_cell)
            .next()
            .is_some_and(|element| normalized_text(element).contains("from"));
        offers.push(Offer {
            plan_key: plan_name.clone(),
            plan_name,
            cpu_cores: first_int(&cpu_text),
            ram_mib: memory_mib(&ram_text),
            disk_gib: disk_gib(&disk_text),
            disk_type: if disk_text.contains("NVMe") {
                Some("NVMe".to_owned())
            } else {
                None
            },
            region: None,
            price_minor: money_minor(&price_text),
            currency: Some("EUR".to_owned()),
            billing_period: Some("month".to_owned()),
            price_kind: if starting_from {
                "starting_from".to_owned()
            } else {
                "regular".to_owned()
            },
            vat_included: Some(false),
            raw_excerpt: excerpt(&format!(
                "{cpu_text}; {ram_text}; {disk_text}; {price_text}"
            )),
        });
    }
    offers
}

fn parse_hetzner_legacy(document: &Html) -> Vec<Offer> {
    let card = selector("div.cputype-toggle");
    let price = selector(".cloud-price-tag .ipv4, .cloud-price-tag");
    let mut offers = Vec::new();
    for card in document.select(&card) {
        let text = normalized_text(card);
        let Some(plan_name) = text.split_whitespace().find(|word| is_hetzner_plan(word)) else {
            continue;
        };
        let price_text = card
            .select(&price)
            .next()
            .map(normalized_text)
            .unwrap_or_default();
        if !price_text.contains('€') {
            continue;
        }
        offers.push(Offer {
            plan_key: plan_name.to_owned(),
            plan_name: plan_name.to_owned(),
            cpu_cores: number_after(&text, "vCPU"),
            ram_mib: number_after(&text, "RAM").map(|value| value * 1024),
            disk_gib: number_after(&text, "Disk space"),
            disk_type: Some("NVMe".to_owned()),
            region: None,
            price_minor: money_minor(&price_text),
            currency: Some("EUR".to_owned()),
            billing_period: Some("month".to_owned()),
            price_kind: "regular_with_ipv4".to_owned(),
            vat_included: Some(false),
            raw_excerpt: excerpt(&text),
        });
    }
    offers
}

fn normalized_text(element: ElementRef<'_>) -> String {
    element
        .text()
        .collect::<Vec<_>>()
        .join(" ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn selector(value: &str) -> Selector {
    Selector::parse(value).expect("static CSS selector")
}

fn first_int(value: &str) -> Option<i32> {
    let digits: String = value
        .chars()
        .skip_while(|character| !character.is_ascii_digit())
        .take_while(char::is_ascii_digit)
        .collect();
    digits.parse().ok()
}

fn memory_mib(value: &str) -> Option<i32> {
    let amount = first_int(value)?;
    if value.to_uppercase().contains("ГБ") || value.to_uppercase().contains("GB") {
        amount.checked_mul(1024)
    } else {
        Some(amount)
    }
}

fn disk_gib(value: &str) -> Option<i32> {
    let amount = first_int(value)?;
    if value.to_uppercase().contains("ТБ") || value.to_uppercase().contains("TB") {
        amount.checked_mul(1024)
    } else {
        Some(amount)
    }
}

fn number_after(value: &str, label: &str) -> Option<i32> {
    let index = value.find(label)?;
    first_int(&value[index + label.len()..])
}

fn is_hetzner_plan(value: &str) -> bool {
    ["CX", "CPX", "CAX", "CCX"].iter().any(|prefix| {
        value.strip_prefix(prefix).is_some_and(|part| {
            part.len() == 2 && part.chars().all(|character| character.is_ascii_digit())
        })
    })
}

fn money_minor(value: &str) -> Option<i64> {
    let raw: String = value
        .chars()
        .skip_while(|character| !character.is_ascii_digit())
        .take_while(|character| {
            character.is_ascii_digit()
                || *character == '.'
                || *character == ','
                || character.is_whitespace()
        })
        .collect();
    let raw = raw.trim();
    let separator = raw.rfind(['.', ',']);
    if let Some(index) = separator {
        let fraction: String = raw[index + 1..]
            .chars()
            .filter(char::is_ascii_digit)
            .collect();
        if fraction.len() == 1 || fraction.len() == 2 {
            let whole: String = raw[..index].chars().filter(char::is_ascii_digit).collect();
            let whole: i64 = whole.parse().ok()?;
            let fraction: i64 = fraction.parse().ok()?;
            return whole.checked_mul(100)?.checked_add(
                if raw[index + 1..]
                    .chars()
                    .filter(char::is_ascii_digit)
                    .count()
                    == 1
                {
                    fraction * 10
                } else {
                    fraction
                },
            );
        }
    }
    let whole: String = raw.chars().filter(char::is_ascii_digit).collect();
    whole.parse::<i64>().ok()?.checked_mul(100)
}

fn excerpt(value: &str) -> String {
    value.chars().take(500).collect()
}
