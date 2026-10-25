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
        "contabo" => {
            let mut offers = parse_contabo(&document);
            offers.extend(parse_contabo_grid(&document));
            offers.extend(parse_contabo_json_ld(&document));
            offers
        }
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
                raw_excerpt: excerpt(&column_details(&rows, column)),
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
            plan_name: name.clone(),
            cpu_cores: first_int(&values[0]),
            ram_mib: memory_mib(&values[1]),
            disk_gib: disk_gib(&values[2]),
            disk_type: Some("NVMe".to_owned()),
            region: if name.starts_with("MSK ") {
                Some("Moscow".to_owned())
            } else {
                None
            },
            price_minor: money_minor(&price_text),
            currency: Some("RUB".to_owned()),
            billing_period: Some("month".to_owned()),
            price_kind: "regular".to_owned(),
            vat_included: None,
            raw_excerpt: excerpt(&normalized_text(row)),
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
                raw_excerpt: excerpt(&column_details(&rows, column)),
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

fn parse_contabo(document: &Html) -> Vec<Offer> {
    let table = selector("table");
    let tr = selector("tr");
    let cell = selector("th, td");
    let mut offers = Vec::new();
    for table in document.select(&table) {
        let rows: Vec<Vec<String>> = table
            .select(&tr)
            .map(|row| row.select(&cell).map(normalized_text).collect())
            .collect();
        let Some(header) = rows.first() else { continue };
        let column = |needle: &str| {
            header
                .iter()
                .position(|value| value.to_lowercase().contains(&needle.to_lowercase()))
        };
        let (Some(model_col), Some(cpu_col), Some(ram_col), Some(storage_col), Some(price_col)) = (
            column("model"),
            column("cpu"),
            column("ram"),
            column("storage"),
            column("price"),
        ) else {
            continue;
        };
        let region_col = column("region").or_else(|| column("location"));
        for row in rows.iter().skip(1) {
            let Some(name) = row.get(model_col).map(|s| s.trim()) else {
                continue;
            };
            if !["Cloud VPS", "Cloud VDS", "Storage VPS"]
                .iter()
                .any(|prefix| name.starts_with(prefix))
            {
                continue;
            }
            let get = |index: usize| row.get(index).map(String::as_str).unwrap_or("");
            let storage = get(storage_col);
            let price_text = get(price_col);
            if !price_text.contains('€') && !price_text.contains('$') {
                continue;
            }
            if let Some(price) = contabo_money_minor(price_text) {
                offers.push(Offer {
                    plan_key: name.to_owned(),
                    plan_name: name.to_owned(),
                    cpu_cores: first_int(get(cpu_col)),
                    ram_mib: memory_mib(get(ram_col)),
                    disk_gib: disk_gib(storage),
                    disk_type: if storage.to_lowercase().contains("nvme") {
                        Some("NVMe".to_owned())
                    } else if storage.to_lowercase().contains("ssd") {
                        Some("SSD".to_owned())
                    } else if storage.to_lowercase().contains("hdd") {
                        Some("HDD".to_owned())
                    } else {
                        None
                    },
                    region: region_col
                        .map(|index| get(index).trim().to_owned())
                        .filter(|region| {
                            !region.is_empty()
                                && !matches!(
                                    region.to_lowercase().as_str(),
                                    "all regions" | "multiple locations" | "global" | "worldwide"
                                )
                        }),
                    price_minor: Some(price),
                    currency: if price_text.contains('€') {
                        Some("EUR".to_owned())
                    } else if price_text.contains('$') {
                        Some("USD".to_owned())
                    } else {
                        None
                    },
                    billing_period: Some("month".to_owned()),
                    price_kind: if row.iter().any(|value| {
                        let lower = value.to_lowercase();
                        lower.contains("sold out") || lower.contains("notify me")
                    }) {
                        "unavailable_or_waitlist".to_owned()
                    } else {
                        "listed_monthly".to_owned()
                    },
                    vat_included: if header[price_col].to_lowercase().contains("including vat") {
                        Some(true)
                    } else if header[price_col].to_lowercase().contains("excluding vat") {
                        Some(false)
                    } else {
                        None
                    },
                    raw_excerpt: excerpt(
                        &row.iter()
                            .enumerate()
                            .map(|(i, value)| {
                                format!("{}: {}", header.get(i).map_or("", String::as_str), value)
                            })
                            .collect::<Vec<_>>()
                            .join("; "),
                    ),
                });
            }
        }
    }
    offers
}

fn contabo_name(name: &str) -> bool {
    ["Cloud VPS", "Cloud VDS", "Storage VPS"]
        .iter()
        .any(|prefix| name.starts_with(prefix))
}

fn contabo_disk_type(value: &str) -> Option<String> {
    let lower = value.to_lowercase();
    if lower.contains("nvme") {
        Some("NVMe".to_owned())
    } else if lower.contains("ssd") {
        Some("SSD".to_owned())
    } else if lower.contains("hdd") {
        Some("HDD".to_owned())
    } else {
        None
    }
}

fn contabo_offer(
    name: String,
    cpu: &str,
    ram: &str,
    disk: &str,
    price: &str,
    details: &str,
) -> Option<Offer> {
    if !contabo_name(&name) || !price.contains("/ month") {
        return None;
    }
    let currency = if price.contains('€') {
        "EUR"
    } else if price.contains('$') {
        "USD"
    } else {
        return None;
    };
    Some(Offer {
        plan_key: name.clone(),
        plan_name: name,
        cpu_cores: first_int(cpu),
        ram_mib: memory_mib(ram),
        disk_gib: disk_gib(disk),
        disk_type: contabo_disk_type(disk),
        region: None,
        price_minor: contabo_money_minor(price),
        currency: Some(currency.to_owned()),
        billing_period: Some("month".to_owned()),
        price_kind: "listed_monthly".to_owned(),
        vat_included: None,
        raw_excerpt: excerpt(details),
    })
}

fn parse_contabo_grid(document: &Html) -> Vec<Offer> {
    let grid = selector(".pricing-table.vps");
    let title = selector(".row-title");
    let spec = selector(".spec-title");
    let mut offers = Vec::new();
    for grid in document.select(&grid) {
        let mut name = None;
        let mut specs = Vec::new();
        let mut details = Vec::new();
        for child in grid.children().filter_map(ElementRef::wrap) {
            if let Some(title) = child.select(&title).next() {
                name = Some(normalized_text(title));
                specs.clear();
                details.clear();
                continue;
            }
            if name.is_none() {
                continue;
            }
            let text = normalized_text(child);
            if let Some(spec) = child.select(&spec).next() {
                specs.push(normalized_text(spec));
            }
            details.push(text.clone());
            if child
                .value()
                .attr("class")
                .is_some_and(|classes| classes.split_whitespace().any(|class| class == "price"))
            {
                let cpu = specs
                    .iter()
                    .find(|item| item.to_lowercase().contains("cpu"));
                let ram = specs
                    .iter()
                    .find(|item| item.to_lowercase().contains("ram"));
                let disk = specs.iter().find(|item| {
                    let lower = item.to_lowercase();
                    lower.contains("nvme") || lower.contains("ssd") || lower.contains("hdd")
                });
                if let (Some(name), Some(cpu), Some(ram), Some(disk)) =
                    (name.take(), cpu, ram, disk)
                {
                    if let Some(offer) =
                        contabo_offer(name, cpu, ram, disk, &text, &details.join("; "))
                    {
                        offers.push(offer);
                    }
                }
            }
        }
    }
    offers
}

fn number_before_marker(value: &str, marker: &str) -> Option<i32> {
    let index = value.to_lowercase().find(&marker.to_lowercase())?;
    value[..index]
        .split(|character: char| !character.is_ascii_digit())
        .filter(|part| !part.is_empty())
        .last()?
        .parse()
        .ok()
}

fn disk_before_kind(description: &str, kind: &str) -> Option<i32> {
    let lower = description.to_lowercase();
    let kind_at = lower.find(kind)?;
    let before = &lower[..kind_at];
    let unit_at = before
        .rfind("gb")
        .into_iter()
        .chain(before.rfind("tb"))
        .max()?;
    let unit = &before[unit_at..unit_at + 2];
    let raw = before[..unit_at].trim_end();
    let amount = raw
        .rsplit(|character: char| {
            !character.is_ascii_digit() && character != '.' && character != ','
        })
        .find(|part| !part.is_empty())?
        .replace(',', ".")
        .parse::<f64>()
        .ok()?;
    let gib = if unit == "tb" {
        amount * 1024.0
    } else {
        amount
    };
    (gib.is_finite() && gib > 0.0 && gib <= i32::MAX as f64).then_some(gib.round() as i32)
}

fn parse_contabo_json_ld(document: &Html) -> Vec<Offer> {
    let scripts = selector("script[type='application/ld+json']");
    let mut offers = Vec::new();
    for script in document.select(&scripts) {
        let body = script.text().collect::<String>();
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&body) else {
            continue;
        };
        if value["@type"] != "Product" {
            continue;
        }
        let (Some(name), Some(description)) =
            (value["name"].as_str(), value["description"].as_str())
        else {
            continue;
        };
        if !contabo_name(name) {
            continue;
        }
        let cpu = [" vcpu", " physical cpu cores", " core", " cores"]
            .iter()
            .find_map(|marker| number_before_marker(description, marker));
        let ram =
            number_before_marker(description, " ram").and_then(|amount| amount.checked_mul(1024));
        let disk = ["nvme", "ssd", "hdd"]
            .iter()
            .find_map(|kind| disk_before_kind(description, kind));
        let Some(price) = value["offers"]
            .as_array()
            .and_then(|offers| offers.iter().find(|offer| offer["priceCurrency"] == "EUR"))
            .and_then(|offer| offer["lowPrice"].as_f64())
        else {
            continue;
        };
        if !price.is_finite() || price <= 0.0 {
            continue;
        }
        offers.push(Offer {
            plan_key: name.to_owned(),
            plan_name: name.to_owned(),
            cpu_cores: cpu,
            ram_mib: ram,
            disk_gib: disk,
            disk_type: contabo_disk_type(description),
            region: None,
            price_minor: Some((price * 100.0).round() as i64),
            currency: Some("EUR".to_owned()),
            billing_period: Some("month".to_owned()),
            price_kind: "listed_monthly".to_owned(),
            vat_included: None,
            raw_excerpt: excerpt(description),
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

fn column_details(rows: &[Vec<String>], column: usize) -> String {
    rows.iter()
        .filter_map(|row| {
            let value = row.get(column)?.trim();
            if value.is_empty() {
                return None;
            }
            let label = row.first().map_or("", String::as_str).trim();
            Some(if label.is_empty() || label == value {
                value.to_owned()
            } else {
                format!("{label}: {value}")
            })
        })
        .collect::<Vec<_>>()
        .join("; ")
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

fn contabo_money_minor(value: &str) -> Option<i64> {
    if value.contains('.') || value.contains(',') {
        return money_minor(value);
    }
    let groups: Vec<i64> = value
        .split(|character: char| !character.is_ascii_digit())
        .filter(|group| !group.is_empty())
        .filter_map(|group| group.parse().ok())
        .collect();
    match groups.as_slice() {
        [major, minor, ..] if *minor < 100 => major.checked_mul(100)?.checked_add(*minor),
        [major, ..] => major.checked_mul(100),
        [] => None,
    }
}

fn excerpt(value: &str) -> String {
    value.chars().take(4000).collect()
}

#[cfg(test)]
mod tests {
    use super::{parse_cdx_json, parse_html};

    #[test]
    fn timeweb_selectel_and_hetzner_extract_prices_and_core_specs() {
        let timeweb = parse_html(
            "timeweb_cloud",
            r#"<div class="row--plan"><div class="titleWrapper"><span>MSK Cloud S</span></div>
              <h5>1 200 ₽ / месяц</h5><div class="wFixed">2 vCPU</div>
              <div class="wFixed">8 GB RAM</div><div class="wFixed">50 GB SSD</div>
              <span>Трафик: 5 ТБ</span></div>"#,
        );
        assert_eq!(timeweb.len(), 1);
        assert!(timeweb[0].is_complete());
        assert_eq!(timeweb[0].price_minor, Some(120_000));
        assert_eq!(timeweb[0].cpu_cores, Some(2));
        assert_eq!(timeweb[0].ram_mib, Some(8192));
        assert_eq!(timeweb[0].disk_gib, Some(50));
        assert_eq!(timeweb[0].region.as_deref(), Some("Moscow"));
        assert!(timeweb[0].raw_excerpt.contains("5 ТБ"));

        let selectel = parse_html(
            "selectel",
            r#"<a class="custom-card"><h5>VDS Standard</h5>
              <table><tr><td>vCPU</td><td>4 cores</td></tr>
              <tr><td>RAM</td><td>16 GB</td></tr><tr><td>Диск</td><td>80 GB SSD</td></tr></table>
              <div class="custom-card__price">2 450 ₽ / month</div></a>"#,
        );
        assert_eq!(selectel.len(), 1);
        assert!(selectel[0].is_complete());
        assert_eq!(selectel[0].price_minor, Some(245_000));
        assert_eq!(selectel[0].ram_mib, Some(16_384));
        assert!(selectel[0].raw_excerpt.contains("80 GB SSD"));

        let hetzner = parse_html(
            "hetzner",
            r#"<div class="cloud-matrix-product"><span class="name-cell">CX22</span>
              <span class="cpu-cell">2 vCPU</span><span class="ram-cell">4 GB RAM</span>
              <span class="drive-cell">40 GB NVMe SSD</span>
              <div class="month-price-cell">from <span class="visually-hidden">€ 3.79</span></div>
            </div>"#,
        );
        assert_eq!(hetzner.len(), 1);
        assert!(hetzner[0].is_complete());
        assert_eq!(hetzner[0].price_minor, Some(379));
        assert_eq!(hetzner[0].disk_type.as_deref(), Some("NVMe"));
        assert_eq!(hetzner[0].price_kind, "starting_from");
    }

    #[test]
    fn parser_rejects_changed_or_incomplete_html_and_deduplicates_repeated_cards() {
        assert!(parse_html("firstvds", "<main>new markup without tariffs</main>").is_empty());
        let no_price = parse_html(
            "contabo",
            r#"<table><tr><th>Model</th><th>CPU</th><th>RAM</th><th>Storage</th><th>Price</th></tr>
              <tr><td>Cloud VPS 4</td><td>4 vCPU</td><td>8 GB</td><td>100 GB SSD</td><td>price unavailable</td></tr></table>"#,
        );
        assert!(
            no_price.is_empty(),
            "malformed prices are not persisted as offers"
        );

        let repeated = r#"<table><tr><th>Model</th><th>CPU</th><th>RAM</th><th>Storage</th><th>Price Including VAT</th></tr>
          <tr><td>Cloud VPS 4</td><td>4 vCPU Cores</td><td>8 GB RAM</td><td>100 GB SSD</td><td>$ 5.28 / month</td></tr>
          <tr><td>Cloud VPS 4</td><td>4 vCPU Cores</td><td>8 GB RAM</td><td>100 GB SSD</td><td>$ 5.28 / month</td></tr></table>"#;
        let offers = parse_html("contabo", repeated);
        assert_eq!(
            offers.len(),
            1,
            "same capture and price do not duplicate an offer"
        );
    }

    #[test]
    fn cdx_parser_handles_column_order_and_rejects_malformed_archive_rows() {
        let captures = parse_cdx_json(
            r#"[["digest","original","timestamp"],["sha-a","https://example.test/vps","20261024010203"],["sha-b","javascript:alert(1)","20261024010203"]]"#,
        )
        .unwrap();
        assert_eq!(captures.len(), 1);
        assert_eq!(captures[0].digest, "sha-a");
        assert_eq!(
            captures[0].replay_url(),
            "https://web.archive.org/web/20261024010203id_/https://example.test/vps"
        );
        assert!(parse_cdx_json("[]").unwrap().is_empty());
        assert!(
            parse_cdx_json(r#"[["original","digest"],["https://example.test","sha"]]"#)
                .unwrap_err()
                .contains("timestamp")
        );
        assert!(
            parse_cdx_json(
                r#"[["timestamp","original","digest"],["not-a-date","https://example.test","sha"]]"#
            )
            .is_err()
        );
    }

    #[test]
    fn firstvds_persists_additional_tariff_rows_with_the_offer() {
        let html = r#"<table>
          <tr><th>Тариф</th><td>VDS-KVM-SSD-Test</td></tr>
          <tr><th>Количество процессоров</th><td>4 ядра</td></tr>
          <tr><th>Оперативная память</th><td>8 ГБ</td></tr>
          <tr><th>Дисковое пространство</th><td>80 ГБ SSD</td></tr>
          <tr><th>Трафик</th><td>Безлимитный</td></tr>
          <tr><th>Стоимость за период: месяц</th><td>1200 ₽</td></tr>
        </table>"#;
        let offers = parse_html("firstvds", html);
        assert_eq!(offers.len(), 1);
        let offer = &offers[0];
        assert!(offer.is_complete());
        assert!(offer.raw_excerpt.contains("Трафик: Безлимитный"));
        assert!(offer.raw_excerpt.contains("Стоимость за период: месяц"));
    }

    #[test]
    fn contabo_parser_captures_price_configuration_and_network_terms() {
        let html = r#"<table><tr><th>Model</th><th>CPU</th><th>RAM</th><th>Storage</th><th>Port</th><th>Data Transfer</th><th>Price Including VAT</th></tr>
          <tr><td>Cloud VPS 4</td><td>4 vCPU Cores</td><td>8 GB RAM</td><td>100 GB SSD</td><td>200 Mbit/s</td><td>Unlimited Traffic</td><td>$ 5 28 / month</td></tr></table>"#;
        let offers = parse_html("contabo", html);
        assert_eq!(offers.len(), 1);
        let offer = &offers[0];
        assert_eq!(offer.cpu_cores, Some(4));
        assert_eq!(offer.ram_mib, Some(8192));
        assert_eq!(offer.disk_gib, Some(100));
        assert_eq!(offer.disk_type.as_deref(), Some("SSD"));
        assert_eq!(offer.price_minor, Some(528));
        assert_eq!(offer.currency.as_deref(), Some("USD"));
        assert!(offer.vat_included.unwrap());
        assert!(offer.raw_excerpt.contains("Unlimited Traffic"));
        assert!(offer.raw_excerpt.contains("200 Mbit/s"));
    }

    #[test]
    fn contabo_archive_grid_reads_monthly_price_and_specs() {
        let html = r#"<div class="pricing-table vps">
          <div><span class="row-title">Cloud VPS 1</span></div>
          <div><span class="spec-title">4 vCPU Cores</span></div>
          <div><span class="spec-title">6 GB RAM</span></div>
          <div><span class="spec-title">100 GB NVMe</span><span>or 400 GB SSD</span></div>
          <div><span class="spec-title">32 TB Traffic</span></div>
          <div class="price">€4.50 <span>/ month</span></div>
        </div>"#;
        let offers = parse_html("contabo", html);
        assert_eq!(offers.len(), 1);
        assert!(offers[0].is_complete());
        assert_eq!(offers[0].price_minor, Some(450));
        assert_eq!(offers[0].disk_type.as_deref(), Some("NVMe"));
    }

    #[test]
    fn contabo_archive_json_ld_reads_decimal_terabyte_disk() {
        let html = r#"<script type="application/ld+json">{
          "@type":"Product","name":"Storage VPS 40",
          "description":"8 vCPU Cores, 30 GB RAM, 1.2 TB SSD, Unlimited Traffic",
          "offers":[{"priceCurrency":"EUR","lowPrice":25.5}]
        }</script>"#;
        let offers = parse_html("contabo", html);
        assert_eq!(offers.len(), 1);
        assert!(offers[0].is_complete());
        assert_eq!(offers[0].disk_gib, Some(1229));
        assert_eq!(offers[0].price_minor, Some(2550));
    }
}
