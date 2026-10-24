use crate::{Plan, money};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::Path, sync::RwLock};
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Fx {
    pub date: String,
    pub source: String,
    pub rub_per_unit: BTreeMap<String, String>,
    pub rounding: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OriginalPrice {
    pub amount: String,
    pub currency: String,
    pub unit: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Details {
    pub original_price: OriginalPrice,
    pub normalized_monthly_usd: String,
    pub normalized_monthly_rub: String,
    pub fx: Option<Fx>,
    pub market: String,
    pub service_type: String,
    pub virtualization: Option<String>,
    pub cpu_class: Option<String>,
    pub vat: String,
    pub complete_monthly_payment: bool,
    pub availability: String,
    pub billing_hours: Option<u32>,
    pub hourly_price: Option<OriginalPrice>,
    pub components: Vec<serde_json::Value>,
    pub source_region: String,
    pub notes: Vec<String>,
    pub egress_gib: Option<u32>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Snapshot {
    #[serde(default)]
    pub contexts: serde_json::Value,
    pub schema_version: u32,
    pub prepared_as_of: String,
    pub catalog_version: String,
    pub fx: Fx,
    pub plans: Vec<Plan>,
    pub aeza_plans: Vec<Plan>,
    pub providers: Vec<serde_json::Value>,
    pub raw_product_count: usize,
    pub excluded_quotes: Vec<serde_json::Value>,
    pub receipts: Vec<serde_json::Value>,
    pub limitations: Vec<String>,
}
#[derive(Deserialize)]
struct Envelope {
    payload: String,
    sha256: String,
}
impl Snapshot {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != 1
            || self.plans.is_empty()
            || self.plans.len() > 1000
            || self.aeza_plans.len() > 1000
        {
            return Err("invalid snapshot schema or size".into());
        }
        if self.fx.source != "https://www.cbr.ru/scripts/XML_daily.asp" || self.fx.date.len() != 10
        {
            return Err("invalid FX provenance".into());
        }
        for code in ["RUB", "USD", "EUR"] {
            let (n, d) = money::ratio(self.fx.rub_per_unit.get(code).ok_or("missing FX rate")?)?;
            if n <= 0 || d <= 0 {
                return Err("nonpositive FX".into());
            }
        }
        if money::decimal(&self.fx.rub_per_unit["RUB"])? != money::SCALE {
            return Err("RUB anchor must equal one".into());
        }
        let mut ids = std::collections::BTreeSet::new();
        for p in self.plans.iter().chain(&self.aeza_plans) {
            if !crate::validate_plan(p) || !ids.insert(&p.id) || !p.source.starts_with("https://") {
                return Err("invalid observed offer".into());
            }
            let meta = p
                .details
                .as_ref()
                .ok_or("snapshot offer missing original price")?;
            let ticks = match &meta.fx {
                Some(fx) => {
                    if serde_json::to_value(fx).unwrap() != serde_json::to_value(&self.fx).unwrap()
                    {
                        return Err("mixed FX snapshots".into());
                    }
                    money::convert(
                        &meta.original_price.amount,
                        &meta.original_price.currency,
                        fx,
                        &meta.original_price.unit,
                        meta.billing_hours,
                    )?
                }
                None => {
                    if meta.original_price.currency != "USD" || meta.original_price.unit != "month"
                    {
                        return Err("conversion missing FX".into());
                    }
                    money::decimal(&meta.original_price.amount)?
                }
            };
            let rub = money::convert_to(
                &meta.original_price.amount,
                &meta.original_price.currency,
                "RUB",
                &self.fx,
                &meta.original_price.unit,
                meta.billing_hours,
            )?;
            if rub != money::decimal(&meta.normalized_monthly_rub)? {
                return Err("RUB normalization mismatch".into());
            }
            if ticks != money::decimal(&meta.normalized_monthly_usd)?
                || money::float(ticks) != p.monthly_usd
            {
                return Err(format!("normalized price mismatch: {}", p.id));
            }
            if p.provider == "aeza" && !self.aeza_plans.iter().any(|x| x.id == p.id) {
                return Err("Aéza in competitor catalog".into());
            }
            if !["vps", "cloud_vm"].contains(&meta.service_type.as_str())
                || !["russia", "international", "unknown"].contains(&meta.market.as_str())
            {
                return Err("unsupported service class/market".into());
            }
        }
        Ok(())
    }
}
pub fn decode(bytes: &[u8]) -> Result<Snapshot, String> {
    let e: Envelope = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    if crate::hash(e.payload.as_bytes()) != e.sha256 {
        return Err("snapshot checksum mismatch".into());
    }
    let snapshot: Snapshot = serde_json::from_str(&e.payload).map_err(|e| e.to_string())?;
    snapshot.validate()?;
    Ok(snapshot)
}
pub fn load(path: &Path) -> Result<Snapshot, String> {
    let meta = std::fs::metadata(path).map_err(|e| e.to_string())?;
    if meta.len() > 20_000_000 {
        return Err("snapshot too large".into());
    }
    decode(&std::fs::read(path).map_err(|e| e.to_string())?)
}
pub fn default_path() -> std::path::PathBuf {
    std::env::var_os("MARKET_DATA_PATH")
        .map(Into::into)
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("data/market-snapshot.json"))
}
/// Reads a single atomically replaced envelope. Failed refresh never destroys the in-memory last good state.
pub struct Store {
    path: std::path::PathBuf,
    last: RwLock<Snapshot>,
}
impl Store {
    pub fn new(path: std::path::PathBuf) -> Self {
        let bundled = decode(include_bytes!("../data/market-snapshot.json"))
            .expect("verified bundled snapshot");
        let initial = load(&path).unwrap_or(bundled);
        Self {
            path,
            last: RwLock::new(initial),
        }
    }
    pub fn current(&self) -> (Snapshot, Option<String>) {
        match load(&self.path) {
            Ok(s) => {
                *self.last.write().unwrap() = s.clone();
                (s, None)
            }
            Err(e) => (
                self.last.read().unwrap().clone(),
                Some(format!("last_verified_snapshot: {e}")),
            ),
        }
    }
}
pub fn runtime() -> Snapshot {
    Store::new(default_path()).current().0
}

pub fn bundled_fx() -> Fx {
    static FX: std::sync::OnceLock<Fx> = std::sync::OnceLock::new();
    FX.get_or_init(|| {
        decode(include_bytes!("../data/market-snapshot.json"))
            .expect("bundled snapshot")
            .fx
    })
    .clone()
}
