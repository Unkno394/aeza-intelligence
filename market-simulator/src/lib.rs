pub mod api;
pub mod statistics;
pub mod behavior;
pub mod competition;
pub mod currency;
pub mod market_data;
pub mod money;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub const VERSION: &str = "market-v1.3.1";
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Plan {
    pub id: String,
    pub provider: String,
    pub name: String,
    pub vcpu: f64,
    pub ram_gb: f64,
    pub disk_gb: f64,
    pub monthly_usd: f64,
    pub regions: Vec<String>,
    pub origin: String,
    pub source: String,
    pub observed_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<market_data::Details>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Segment {
    pub name: String,
    pub weight: f64,
    pub budget_min: f64,
    pub budget_max: f64,
    pub vcpu: f64,
    pub ram_gb: f64,
    pub disk_gb: f64,
    /// None accepts unknown locations. Some requires an explicit catalog match.
    pub region: Option<String>,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Sensitivity {
    Low,
    Medium,
    High,
}
impl Sensitivity {
    pub fn beta(self) -> f64 {
        match self {
            Self::Low => 0.5,
            Self::Medium => 2.0,
            Self::High => 5.0,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Strategy {
    pub name: String,
    pub aeza_price_multipliers: Vec<f64>,
}
/// Scenario interventions on existing observed plans, never replacement observations.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MarketEvent {
    pub plan_id: String,
    pub starts_month: usize,
    pub price_multiplier: f64,
    pub available: bool,
    /// None applies to all strategies; Some isolates a controlled comparison.
    pub strategy: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Flow {
    pub from: String,
    pub to: String,
    pub events: usize,
    pub unique_buyers: usize,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Decision {
    pub buyer_id: usize,
    pub segment: String,
    pub provider_preference: Option<f64>,
    pub price_penalty: Option<f64>,
    pub switching_penalty: Option<f64>,
    pub from: String,
    pub to: String,
    pub reason: String,
    pub budget_usd: f64,
    pub chosen_plan_id: Option<String>,
    pub effective_price_usd: Option<f64>,
    pub utility: Option<f64>,
    pub previous_offer_utility: Option<f64>,
    pub outside_utility: f64,
    pub best_competitor_plan_id: Option<String>,
    pub best_competitor_utility: Option<f64>,
    pub eligible_offers: usize,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Capacity {
    pub vcpu: f64,
    pub ram_gb: f64,
    pub storage_gb: f64,
    pub origin: String,
    pub source: String,
}
fn default_capacity() -> Capacity {
    Capacity {
        vcpu: 10000.,
        ram_gb: 20000.,
        storage_gb: 500000.,
        origin: "scenario_assumption".into(),
        source: "scenario://default-logical-resource-pool-not-observed-stock".into(),
    }
}
fn default_currency() -> String {
    "USD".into()
}
fn default_market() -> String {
    "all".into()
}
fn default_services() -> Vec<String> {
    vec!["vps".into()]
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CapacityUsage {
    pub vcpu: f64,
    pub ram_gb: f64,
    pub storage_gb: f64,
    pub decisions_blocked: usize,
}
/// An external model result, not a model implementation. Two slots have independent provenance.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelOutput {
    pub slot: String,
    pub model_version: String,
    pub source: String,
    pub schema_version: u32,
    /// Only an explicitly calibrated departure probability is accepted.
    pub semantics: String,
    pub monthly_departure_probability: Vec<f64>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scenario {
    #[serde(default = "default_currency")]
    pub currency: String,
    #[serde(default)]
    pub valuation_fx: Option<market_data::Fx>,
    pub seed: u64,
    pub months: usize,
    pub buyers_per_month: usize,
    pub arrival_weights: Vec<f64>,
    pub segments: Vec<Segment>,
    pub aeza_plans: Vec<Plan>,
    pub strategies: Vec<Strategy>,
    pub sensitivities: Vec<Sensitivity>,
    pub monthly_departure_probability: f64,
    pub switching_cost_utility: f64,
    pub outside_option_utility: f64,
    pub preference_strength: f64,
    pub aeza_variable_cost_usd: f64,
    pub aeza_fixed_cost_usd: f64,
    pub external_models: Vec<ModelOutput>,
    #[serde(default)]
    pub market_events: Vec<MarketEvent>,
    #[serde(default = "default_market")]
    pub market: String,
    #[serde(default = "default_services")]
    pub accepted_service_types: Vec<String>,
    #[serde(default)]
    pub required_virtualization: Option<String>,
    #[serde(default)]
    pub required_cpu_class: Option<String>,
    #[serde(default)]
    pub require_complete_monthly_payment: bool,
    #[serde(default = "default_capacity")]
    pub aeza_capacity: Capacity,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ProviderMetrics {
    #[serde(default)]
    pub at_risk: usize,
    #[serde(default)]
    pub first_leases: usize,
    #[serde(default)]
    pub returning_leases: usize,
    pub active_leases: usize,
    pub new_leases: usize,
    pub renewals: usize,
    pub switches_in: usize,
    pub switches_out: usize,
    pub churn_to_outside: usize,
    pub revenue_usd: f64,
    pub market_share: f64,
    pub cost_usd: Option<f64>,
    pub profit_usd: Option<f64>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Month {
    #[serde(default)]
    pub buyer_behavior: Vec<behavior::SegmentMonth>,
    pub month: usize,
    /// Actual per-plan prices from the same vector used for choices and billing.
    #[serde(default)]
    pub aeza_effective_prices_usd: BTreeMap<String, f64>,
    #[serde(default)]
    pub effective_prices_usd: BTreeMap<String, f64>,
    #[serde(default)]
    pub plan_active_leases: BTreeMap<String, usize>,
    #[serde(default)]
    pub flows: Vec<Flow>,
    #[serde(default)]
    pub decisions: Vec<Decision>,
    #[serde(default)]
    pub aeza_capacity_usage: CapacityUsage,
    #[serde(default)]
    pub billing_ledger_usd: BTreeMap<String, BTreeMap<String, String>>,
    #[serde(default)]
    pub exact_prices_usd: BTreeMap<String, String>,
    pub arrivals: usize,
    pub cumulative_buyers: usize,
    pub active_leases: usize,
    pub outside_buyers: usize,
    pub providers: BTreeMap<String, ProviderMetrics>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Run {
    #[serde(default)]
    pub behavior_unique_clients: BTreeMap<String, usize>,
    pub strategy: String,
    pub sensitivity: Sensitivity,
    pub months: Vec<Month>,
    #[serde(default)]
    pub flows: Vec<Flow>,
    #[serde(default)]
    pub exact_totals_usd: BTreeMap<String, String>,
    pub aeza_total_revenue_usd: f64,
    pub aeza_total_profit_usd: f64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Report {
    pub engine_version: String,
    pub replay_sha256: String,
    pub catalog_sha256: String,
    pub scenario: Scenario,
    pub warnings: Vec<String>,
    pub runs: Vec<Run>,
}
pub fn catalog() -> Vec<Plan> {
    serde_json::from_str(include_str!("../data/catalog.json")).expect("bundled catalog")
}
pub fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn finite_range(x: f64, lo: f64, hi: f64) -> bool {
    x.is_finite() && x >= lo && x <= hi
}
fn positive(x: f64) -> bool {
    finite_range(x, 0.000001, 1e9)
}
fn validate_plan(p: &Plan) -> bool {
    !p.id.is_empty()
        && !p.provider.is_empty()
        && !p.source.is_empty()
        && !p.origin.is_empty()
        && [p.vcpu, p.ram_gb, p.disk_gb, p.monthly_usd]
            .into_iter()
            .all(positive)
}
impl Scenario {
    pub fn validate(&self, market: &[Plan]) -> Result<(), String> {
        let fail = |s: &str| Err(s.to_owned());
        if !["USD", "RUB"].contains(&self.currency.as_str()) {
            return fail("scenario currency must be RUB (v2) or legacy USD (v1)");
        }
        if let Some(fx) = &self.valuation_fx {
            for code in ["USD", "EUR", "RUB"] {
                let Some(rate) = fx.rub_per_unit.get(code) else {
                    return fail("missing valuation rate");
                };
                let Ok((n, d)) = money::ratio(rate) else {
                    return fail("invalid valuation rate");
                };
                if n <= 0 || d <= 0 || n as f64 / d as f64 > 1e9 {
                    return fail("valuation rate out of range");
                }
            }
            if fx.source != "https://www.cbr.ru/scripts/XML_daily.asp" || fx.date.len() != 10 {
                return fail("unverified FX provenance");
            }
        }

        if self.months == 0 || self.months > 24 || self.buyers_per_month > 1000 {
            return fail("months: 1..24; buyers_per_month: 0..1000");
        }
        if market.is_empty()
            || market.len() > 1000
            || market
                .iter()
                .any(|p| !validate_plan(p) || p.provider == "aeza")
        {
            return fail("invalid competitor catalog");
        }
        if self.aeza_plans.is_empty()
            || self.aeza_plans.len() > 20
            || self
                .aeza_plans
                .iter()
                .any(|p| !validate_plan(p) || p.provider != "aeza")
        {
            return fail("1..20 valid aeza plans required");
        }
        let mut ids = BTreeSet::new();
        if market
            .iter()
            .chain(&self.aeza_plans)
            .any(|p| !ids.insert(&p.id))
        {
            return fail("duplicate plan id");
        }
        if self.strategies.is_empty()
            || self.strategies.len() > 5
            || self.sensitivities.is_empty()
            || self.sensitivities.len() > 3
        {
            return fail("1..5 strategies; 1..3 sensitivities");
        }
        let work = self.months * (self.months + 1) / 2
            * self.buyers_per_month
            * 2
            * (market.len() + self.aeza_plans.len())
            * self.strategies.len()
            * self.sensitivities.len();
        if work > 50_000_000 {
            return fail(
                "scenario exceeds 50 million candidate evaluations; reduce population, horizon or comparisons",
            );
        }
        let mut names = BTreeSet::new();
        for s in &self.strategies {
            if s.name.is_empty()
                || !names.insert(&s.name)
                || s.aeza_price_multipliers.len() != self.months
                || s.aeza_price_multipliers
                    .iter()
                    .any(|x| !finite_range(*x, 0.05, 20.))
            {
                return fail("unique strategies require one multiplier (0.05..20) per month");
            }
        }
        if self
            .sensitivities
            .iter()
            .enumerate()
            .any(|(i, x)| self.sensitivities[..i].contains(x))
        {
            return fail("duplicate sensitivity");
        }
        if self.arrival_weights.len() != self.months
            || self
                .arrival_weights
                .iter()
                .any(|x| !finite_range(*x, 0., 2.))
        {
            return fail("one arrival weight (0..2) per month required");
        }
        if self.segments.is_empty()
            || self.segments.len() > 20
            || self.segments.iter().any(|s| {
                !positive(s.weight)
                    || !positive(s.budget_min)
                    || !positive(s.budget_max)
                    || s.budget_min > s.budget_max
                    || ![s.vcpu, s.ram_gb, s.disk_gb].into_iter().all(positive)
            })
        {
            return fail("invalid segments");
        }
        if !finite_range(self.monthly_departure_probability, 0., 1.)
            || !finite_range(self.switching_cost_utility, 0., 20.)
            || !finite_range(self.preference_strength, 0., 20.)
            || !finite_range(self.outside_option_utility, -20., 20.)
            || !finite_range(self.aeza_variable_cost_usd, 0., 1e6)
            || !finite_range(self.aeza_fixed_cost_usd, 0., 1e9)
        {
            return fail("invalid behavioral or cost assumptions");
        }
        if self.market_events.len() > 100 {
            return fail("maximum 100 market events");
        }
        let mut event_keys = BTreeSet::new();
        for e in &self.market_events {
            if !market.iter().any(|p| p.id == e.plan_id)
                || e.starts_month == 0
                || e.starts_month > self.months
                || !finite_range(e.price_multiplier, 0.05, 20.)
                || e.strategy
                    .as_ref()
                    .is_some_and(|name| !self.strategies.iter().any(|s| &s.name == name))
                || !event_keys.insert((&e.plan_id, &e.strategy, e.starts_month))
            {
                return fail(
                    "invalid market event: require existing competitor ID, month, multiplier and strategy; no duplicate schedule entries",
                );
            }
        }
        // Global and strategy-specific events cannot overlap for the same plan.
        for e in &self.market_events {
            if e.strategy.is_none()
                && self
                    .market_events
                    .iter()
                    .any(|x| x.plan_id == e.plan_id && x.strategy.is_some())
            {
                return fail("ambiguous global and strategy-specific plan events");
            }
        }
        if !["all", "russia", "international"].contains(&self.market.as_str())
            || self.accepted_service_types.is_empty()
            || self
                .accepted_service_types
                .iter()
                .any(|s| !["vps", "cloud_vm"].contains(&s.as_str()))
            || self
                .required_virtualization
                .as_ref()
                .is_some_and(|s| s.is_empty() || s.len() > 40)
            || self.required_cpu_class.as_ref().is_some_and(|s| {
                !["shared", "dedicated", "100_percent_catalog_compute"].contains(&s.as_str())
            })
        {
            return fail("invalid market, service types or virtualization/CPU requirement");
        }
        if [
            self.aeza_capacity.vcpu,
            self.aeza_capacity.ram_gb,
            self.aeza_capacity.storage_gb,
        ]
        .iter()
        .any(|x| !finite_range(*x, 0., 1e9))
            || !["scenario_assumption", "observed_capacity"]
                .contains(&self.aeza_capacity.origin.as_str())
            || self.aeza_capacity.source.is_empty()
        {
            return fail("explicit nonnegative resource capacity and provenance required");
        }
        let mut slots = BTreeSet::new();
        if self.external_models.len() > 2 {
            return fail("maximum two external model slots");
        }
        for m in &self.external_models {
            if !["snapshot", "historical"].contains(&m.slot.as_str())
                || !slots.insert(&m.slot)
                || m.schema_version != 1
                || m.semantics != "calibrated_monthly_departure_probability"
                || m.model_version.is_empty()
                || m.source.is_empty()
                || m.monthly_departure_probability.len() != self.months
                || m.monthly_departure_probability
                    .iter()
                    .any(|x| !finite_range(*x, 0., 1.))
            {
                return fail(
                    "invalid model result: require calibrated probabilities with provenance, not raw risk/familiarity scores",
                );
            }
        }
        Ok(())
    }
}
/// Fixed SplitMix64 keyed draws. No hash-map iteration or branch-dependent PRNG stream.
fn draw(seed: u64, buyer: usize, month: usize, channel: u64) -> f64 {
    let mut z = seed.wrapping_add((buyer as u64).wrapping_mul(0x9e3779b97f4a7c15))
        ^ (month as u64).wrapping_mul(0xbf58476d1ce4e5b9)
        ^ channel.wrapping_mul(0x94d049bb133111eb);
    z = z.wrapping_add(0x9e3779b97f4a7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
    ((z ^ (z >> 31)) >> 11) as f64 / ((1u64 << 53) as f64)
}
#[derive(Clone)]
struct Buyer {
    id: usize,
    segment: usize,
    budget: f64,
    plan: Option<usize>,
    ever_active: bool,
}
fn buyer(s: &Scenario, id: usize) -> Buyer {
    let total: f64 = s.segments.iter().map(|s| s.weight).sum();
    let r = draw(s.seed, id, 0, 0) * total;
    let mut sum = 0.;
    let segment = s
        .segments
        .iter()
        .position(|g| {
            sum += g.weight;
            sum > r
        })
        .unwrap_or(s.segments.len() - 1);
    let g = &s.segments[segment];
    Buyer {
        id,
        segment,
        budget: g.budget_min + draw(s.seed, id, 0, 1) * (g.budget_max - g.budget_min),
        plan: None,
        ever_active: false,
    }
}
fn preference(seed: u64, buyer: usize, provider: &str) -> f64 {
    // Explicit byte hash, stable across Rust releases.
    let h = provider.bytes().fold(14695981039346656037u64, |h, b| {
        (h ^ b as u64).wrapping_mul(1099511628211)
    });
    draw(seed, buyer, 0, h)
}
fn policy_eligible(s: &Scenario, plan: &Plan, price: f64) -> bool {
    let details = plan.details.as_ref();
    price.is_finite()
        && details.is_none_or(|d| d.availability != "unavailable")
        && (s.market == "all" || details.is_some_and(|d| d.market == s.market))
        && s.accepted_service_types
            .iter()
            .any(|t| t == details.map_or("vps", |d| d.service_type.as_str()))
        && s.required_virtualization
            .as_ref()
            .is_none_or(|v| details.is_some_and(|d| d.virtualization.as_ref() == Some(v)))
        && s.required_cpu_class
            .as_ref()
            .is_none_or(|v| details.is_some_and(|d| d.cpu_class.as_ref() == Some(v)))
        && (!s.require_complete_monthly_payment
            || details.is_some_and(|d| d.complete_monthly_payment))

}
fn eligible(s: &Scenario, b: &Buyer, plan: &Plan, price: f64) -> bool {
    let g = &s.segments[b.segment];
    policy_eligible(s,plan,price)
        && price <= b.budget
        && plan.vcpu >= g.vcpu
        && plan.ram_gb >= g.ram_gb
        && plan.disk_gb >= g.disk_gb
        && g.region
            .as_ref()
            .is_none_or(|r| competition::region_matches(&plan.regions, r))
}
#[cfg(test)]
fn choose(
    s: &Scenario,
    b: &Buyer,
    plans: &[Plan],
    prices: &[f64],
    sensitivity: Sensitivity,
) -> Option<usize> {
    choose_available(s, b, plans, prices, sensitivity, None).0
}
fn base_money(p: &Plan) -> i128 {
    if let Some(d) = &p.details {
        if let Ok(v) = money::decimal(&d.normalized_monthly_rub) {
            if p.monthly_usd == money::float(v) {
                return v;
            }
        }
    }
    money::ticks(p.monthly_usd)
}
fn resources(p: &Plan) -> [i128; 3] {
    [
        money::ticks(p.vcpu),
        money::ticks(p.ram_gb),
        money::ticks(p.disk_gb),
    ]
}
fn fits(p: &Plan, free: [i128; 3]) -> bool {
    resources(p).iter().zip(free).all(|(v, cap)| *v <= cap)
}
fn choose_available(
    s: &Scenario,
    b: &Buyer,
    plans: &[Plan],
    prices: &[f64],
    sensitivity: Sensitivity,
    free: Option<[i128; 3]>,
) -> (Option<usize>, bool) {
    let mut blocked = false;
    let mut best = s.outside_option_utility;
    let mut choice = None;
    for (i, p) in plans.iter().enumerate() {
        if !eligible(s, b, p, prices[i]) {
            continue;
        }
        if p.provider == "aeza" && free.is_some_and(|cap| !fits(p, cap)) {
            blocked = true;
            continue;
        }
        let score = utility(s, b, plans, prices, sensitivity, i);
        if score > best {
            best = score;
            choice = Some(i)
        }
    }
    (choice, blocked)
}
fn price_penalty(sensitivity: Sensitivity, price: f64, budget: f64) -> f64 {
    let (n, d) = match sensitivity {
        Sensitivity::Low => (1, 2),
        Sensitivity::Medium => (2, 1),
        Sensitivity::High => (5, 1),
    };
    (money::ticks(price) * n) as f64 / (money::ticks(budget) * d) as f64
}
fn utility(
    s: &Scenario,
    b: &Buyer,
    plans: &[Plan],
    prices: &[f64],
    sensitivity: Sensitivity,
    i: usize,
) -> f64 {
    2. + s.preference_strength * preference(s.seed, b.id, &plans[i].provider)
        - price_penalty(sensitivity, prices[i], b.budget)
        - if b
            .plan
            .is_some_and(|old| plans[old].provider != plans[i].provider)
        {
            s.switching_cost_utility
        } else {
            0.
        }
}
fn flows_from(map: BTreeMap<(String, String), (usize, BTreeSet<usize>)>) -> Vec<Flow> {
    map.into_iter()
        .map(|((from, to), (events, ids))| Flow {
            from,
            to,
            events,
            unique_buyers: ids.len(),
        })
        .collect()
}
pub fn simulate(s: Scenario, market: Vec<Plan>) -> Result<Report, String> {
    s.validate(&market)?;
    if s.currency == "USD" {
        let fx = s
            .valuation_fx
            .clone()
            .unwrap_or_else(market_data::bundled_fx);
        let native = currency::scenario_to_rub(s.clone(), &fx);
        let market = market
            .into_iter()
            .map(|p| currency::plan_to_rub(p, &fx))
            .collect();
        let r = simulate_rub(native, market)?;
        Ok(currency::legacy_report(r, s, &fx))
    } else {
        simulate_rub(s, market)
    }
}
fn simulate_rub(s: Scenario, mut market: Vec<Plan>) -> Result<Report, String> {
    s.validate(&market)?;
    market.sort_by(|a, b| a.id.cmp(&b.id));
    let catalog_sha256 = hash(&serde_json::to_vec(&market).unwrap());
    let replay_sha256 = hash(&serde_json::to_vec(&(VERSION, &s, &catalog_sha256)).unwrap());
    let mut aeza = s.aeza_plans.clone();
    aeza.sort_by(|a, b| a.id.cmp(&b.id));
    market.extend(aeza);
    // Apply documented tie-breaking across all providers, including Aéza.
    market.sort_by(|a, b| a.id.cmp(&b.id));
    let mut runs = Vec::new();
    for strategy in &s.strategies {
        for &sensitivity in &s.sensitivities {
            let mut buyers: Vec<Buyer> = Vec::new();
            let mut months = Vec::new();
            let mut behavior_ids: BTreeMap<String, BTreeSet<usize>> = BTreeMap::new();
            let mut run_flows: BTreeMap<(String, String), (usize, BTreeSet<usize>)> =
                BTreeMap::new();
            let mut revenue = 0i128;
            let mut profit = 0i128;
            for month in 0..s.months {
                let price_ticks: Vec<Option<i128>> = market
                    .iter()
                    .map(|p| {
                        if p.details
                            .as_ref()
                            .is_some_and(|d| d.availability == "unavailable")
                        {
                            return None;
                        }
                        let base = base_money(p);
                        if p.provider == "aeza" {
                            Some(money::apply(base, strategy.aeza_price_multipliers[month]))
                        } else {
                            let event = s
                                .market_events
                                .iter()
                                .filter(|e| {
                                    e.plan_id == p.id
                                        && e.starts_month <= month + 1
                                        && e.strategy.as_ref().is_none_or(|n| n == &strategy.name)
                                })
                                .max_by_key(|e| e.starts_month);
                            match event {
                                Some(e) if !e.available => None,
                                Some(e) => Some(money::apply(base, e.price_multiplier)),
                                None => Some(base),
                            }
                        }
                    })
                    .collect();
                let prices: Vec<f64> = price_ticks
                    .iter()
                    .map(|v| v.map(money::float).unwrap_or(f64::INFINITY))
                    .collect();
                let mut month_flows: BTreeMap<(String, String), (usize, BTreeSet<usize>)> =
                    BTreeMap::new();
                let mut decisions = Vec::new();
                let mut behavior_rows: BTreeMap<usize, behavior::SegmentMonth> = BTreeMap::new();
                let limits = [
                    money::ticks(s.aeza_capacity.vcpu),
                    money::ticks(s.aeza_capacity.ram_gb),
                    money::ticks(s.aeza_capacity.storage_gb),
                ];
                let mut used = [0i128; 3];
                for b in &buyers {
                    if let Some(i) = b.plan {
                        if market[i].provider == "aeza" {
                            let r = resources(&market[i]);
                            for k in 0..3 {
                                used[k] += r[k];
                            }
                        }
                    }
                }
                let mut capacity_blocked = 0;
                let mut month_revenue: BTreeMap<String, i128> = BTreeMap::new();
                let mut plan_active_leases = BTreeMap::new();
                let mut providers: BTreeMap<String, ProviderMetrics> = market
                    .iter()
                    .map(|p| (p.provider.clone(), ProviderMetrics::default()))
                    .collect();
                let arrivals =
                    (s.buyers_per_month as f64 * s.arrival_weights[month]).round() as usize;
                let old_len = buyers.len();
                for id in old_len..old_len + arrivals {
                    buyers.push(buyer(&s, id));
                }
                // Model slots combine by equal arithmetic mean, explicitly a scenario fusion rule.
                let departure = if s.external_models.is_empty() {
                    s.monthly_departure_probability
                } else {
                    s.external_models
                        .iter()
                        .map(|m| m.monthly_departure_probability[month])
                        .sum::<f64>()
                        / s.external_models.len() as f64
                };
                for b in &mut buyers {
                    let old = b.plan;
                    if let Some(i) = old {
                        if market[i].provider == "aeza" {
                            let r = resources(&market[i]);
                            for k in 0..3 {
                                used[k] -= r[k];
                            }
                        }
                    }
                    if let Some(i) = old {
                        providers.get_mut(&market[i].provider).unwrap().at_risk += 1;
                    }
                    let forced_departure =
                        old.is_some() && draw(s.seed, b.id, month, 2) < departure;
                    // A departure is absorbing for this billing period; outside buyers retry next month.
                    let next = if forced_departure {
                        None
                    } else {
                        let free = [
                            limits[0] - used[0],
                            limits[1] - used[1],
                            limits[2] - used[2],
                        ];
                        let (choice, blocked) =
                            choose_available(&s, b, &market, &prices, sensitivity, Some(free));
                        capacity_blocked += usize::from(blocked);
                        choice
                    };
                    let free = [limits[0]-used[0],limits[1]-used[1],limits[2]-used[2]];
                    let reason = behavior::classify(&s,b,&market,&prices,sensitivity,free,next,forced_departure);
                    let row = behavior_rows.entry(b.segment).or_insert_with(|| behavior::SegmentMonth {segment_index:b.segment,segment:s.segments[b.segment].name.clone(),..Default::default()});
                    let segment_aeza_revenue = next
                        .filter(|i| market[*i].provider == "aeza")
                        .map(|i| money::float(price_ticks[i].expect("chosen offer has a billable price")))
                        .unwrap_or(0.);
                    row.record(b, &market, next, forced_departure, reason, segment_aeza_revenue);
                    behavior_ids.entry(reason.into()).or_default().insert(b.id);
                    let from = old.map_or("outside", |i| market[i].provider.as_str());
                    let to = next.map_or("outside", |i| market[i].provider.as_str());
                    if from != to {
                        for flow_map in [&mut month_flows, &mut run_flows] {
                            let entry = flow_map
                                .entry((from.to_owned(), to.to_owned()))
                                .or_default();
                            entry.0 += 1;
                            entry.1.insert(b.id);
                        }
                    }
                    // Bounded examples use the very same choices, prices and utility as billing.
                    if decisions.len() < 8 && (from != to || decisions.is_empty()) {
                        let best_competitor = market
                            .iter()
                            .enumerate()
                            .filter(|(i, p)| p.provider != "aeza" && eligible(&s, b, p, prices[*i]))
                            .max_by(|(i, _), (j, _)| {
                                utility(&s, b, &market, &prices, sensitivity, *i)
                                    .total_cmp(&utility(&s, b, &market, &prices, sensitivity, *j))
                                    .then_with(|| j.cmp(i))
                            })
                            .map(|(i, _)| i);
                        decisions.push(Decision {
                            buyer_id: b.id,
                            segment: s.segments[b.segment].name.clone(),
                            provider_preference: next.map(|i| {
                                s.preference_strength
                                    * preference(s.seed, b.id, &market[i].provider)
                            }),
                            price_penalty: next
                                .map(|i| price_penalty(sensitivity, prices[i], b.budget)),
                            switching_penalty: next.map(|i| {
                                if b.plan
                                    .is_some_and(|old| market[old].provider != market[i].provider)
                                {
                                    s.switching_cost_utility
                                } else {
                                    0.
                                }
                            }),
                            from: from.into(),
                            to: to.into(),
                            reason: if forced_departure {
                                "exogenous_departure"
                            } else if next.is_none() {
                                "no_eligible_offer_above_outside_utility"
                            } else if old.is_some() && from == to {
                                "renewal_retained"
                            } else if old.is_some() {
                                "renewal_utility_choice"
                            } else if b.ever_active {
                                "return_utility_choice"
                            } else {
                                "first_utility_choice"
                            }
                            .into(),
                            budget_usd: b.budget,
                            chosen_plan_id: next.map(|i| market[i].id.clone()),
                            effective_price_usd: next.map(|i| prices[i]),
                            utility: next.map(|i| utility(&s, b, &market, &prices, sensitivity, i)),
                            previous_offer_utility: old
                                .filter(|i| prices[*i].is_finite() && prices[*i] <= b.budget)
                                .map(|i| utility(&s, b, &market, &prices, sensitivity, i)),
                            outside_utility: s.outside_option_utility,
                            best_competitor_plan_id: best_competitor.map(|i| market[i].id.clone()),
                            best_competitor_utility: best_competitor
                                .map(|i| utility(&s, b, &market, &prices, sensitivity, i)),
                            eligible_offers: market
                                .iter()
                                .enumerate()
                                .filter(|(i, p)| eligible(&s, b, p, prices[*i]))
                                .count(),
                        });
                    }
                    if let Some(i) = old {
                        let m = providers.get_mut(&market[i].provider).unwrap();
                        match next {
                            None => m.churn_to_outside += 1,
                            Some(j) if market[j].provider != market[i].provider => {
                                m.switches_out += 1
                            }
                            _ => {}
                        }
                    }
                    if let Some(i) = next {
                        let m = providers.get_mut(&market[i].provider).unwrap();
                        m.active_leases += 1;
                        *plan_active_leases.entry(market[i].id.clone()).or_insert(0) += 1;
                        *month_revenue.entry(market[i].provider.clone()).or_default() +=
                            price_ticks[i].expect("chosen offer has a billable price");
                        if market[i].provider == "aeza" {
                            let r = resources(&market[i]);
                            for k in 0..3 {
                                used[k] += r[k];
                            }
                        }
                        match old {
                            None => {
                                m.new_leases += 1;
                                if b.ever_active {
                                    m.returning_leases += 1;
                                } else {
                                    m.first_leases += 1;
                                }
                            }
                            Some(j) if market[j].provider == market[i].provider => m.renewals += 1,
                            Some(_) => m.switches_in += 1,
                        }
                    }
                    b.ever_active |= next.is_some();
                    b.plan = next;
                }
                let active: usize = providers.values().map(|m| m.active_leases).sum();
                let mut ledger = BTreeMap::new();
                for (provider, m) in &mut providers {
                    let billed = *month_revenue.get(provider).unwrap_or(&0);
                    m.revenue_usd = money::float(billed);
                    let mut amounts = BTreeMap::from([("revenue".into(), money::text(billed))]);
                    m.market_share = if active == 0 {
                        0.
                    } else {
                        m.active_leases as f64 / active as f64
                    };
                    if provider == "aeza" {
                        let cost = money::ticks(s.aeza_fixed_cost_usd)
                            + money::ticks(s.aeza_variable_cost_usd) * m.active_leases as i128;
                        m.cost_usd = Some(money::float(cost));
                        m.profit_usd = Some(money::float(billed - cost));
                        revenue += billed;
                        profit += billed - cost;
                        amounts.insert("cost".into(), money::text(cost));
                        amounts.insert("profit".into(), money::text(billed - cost));
                    }
                    ledger.insert(provider.clone(), amounts);
                }
                months.push(Month {
                    buyer_behavior: behavior_rows.into_values().map(|mut r|{r.loss_rate=(r.aeza_at_risk>0).then(||(r.switches_out+r.exogenous_exits+r.choice_exits) as f64/r.aeza_at_risk as f64);r}).collect(),
                    month: month + 1,
                    aeza_effective_prices_usd: market
                        .iter()
                        .enumerate()
                        .filter(|(i, p)| p.provider == "aeza" && price_ticks[*i].is_some())
                        .map(|(i, p)| (p.id.clone(), prices[i]))
                        .collect(),
                    effective_prices_usd: market
                        .iter()
                        .enumerate()
                        .filter(|(i, _)| prices[*i].is_finite())
                        .map(|(i, p)| (p.id.clone(), prices[i]))
                        .collect(),
                    plan_active_leases,
                    flows: flows_from(month_flows),
                    decisions,
                    aeza_capacity_usage: CapacityUsage {
                        vcpu: money::float(used[0]),
                        ram_gb: money::float(used[1]),
                        storage_gb: money::float(used[2]),
                        decisions_blocked: capacity_blocked,
                    },
                    billing_ledger_usd: ledger,
                    exact_prices_usd: market
                        .iter()
                        .enumerate()
                        .filter_map(|(i, p)| price_ticks[i].map(|v| (p.id.clone(), money::text(v))))
                        .collect(),
                    arrivals,
                    cumulative_buyers: buyers.len(),
                    active_leases: active,
                    outside_buyers: buyers.len() - active,
                    providers,
                });
            }
            runs.push(Run {
                behavior_unique_clients: behavior_ids.into_iter().map(|(k,v)|(k,v.len())).collect(),
                flows: flows_from(run_flows),
                strategy: strategy.name.clone(),
                sensitivity,
                months,
                exact_totals_usd: BTreeMap::from([
                    ("revenue".into(), money::text(revenue)),
                    ("profit".into(), money::text(profit)),
                ]),
                aeza_total_revenue_usd: money::float(revenue),
                aeza_total_profit_usd: money::float(profit),
            });
        }
    }
    Ok(Report{engine_version:VERSION.into(),replay_sha256,catalog_sha256,scenario:s,warnings:vec!["Scenario simulation, not a validated forecast. Market share covers this synthetic market only.".into(),"Original observed Aéza prices have provenance; edited prices, budgets, costs, capacity limits, preferences, departures and Akash transfer are scenario assumptions.".into(),"Unknown regions excluded for region-constrained buyers; stock, taxes, CPU performance, architecture and IP not verified.".into(),"Each active lease is billed once per synthetic month at current strategy price; no proration or grandfathering.".into()],runs})
}
pub fn example() -> Scenario {
    serde_json::from_str(include_str!("../data/example.json")).expect("example")
}

#[cfg(test)]
mod price_choice_regressions {
    use super::*;
    #[test]
    fn own_price_direction_for_identical_individual_buyers_and_incumbents() {
        let s = example();
        assert_eq!(s.seed, 42);
        let mut plans = catalog();
        plans.extend(s.aeza_plans.clone());
        plans.sort_by(|a, b| a.id.cmp(&b.id));
        let aeza = plans.iter().position(|p| p.provider == "aeza").unwrap();
        let competitor = plans.iter().position(|p| p.provider != "aeza").unwrap();
        let base: Vec<f64> = plans.iter().map(|p| p.monthly_usd).collect();
        for sensitivity in [Sensitivity::Low, Sensitivity::Medium, Sensitivity::High] {
            for incumbent in [None, Some(aeza), Some(competitor)] {
                for id in 0..100 {
                    let mut b = buyer(&s, id);
                    b.plan = incumbent;
                    let originally_aeza = choose(&s, &b, &plans, &base, sensitivity)
                        .is_some_and(|i| plans[i].provider == "aeza");
                    for factor in [0.6, 0.72, 0.8, 0.85, 0.9, 1.1, 1.15, 1.2, 1.36, 1.4] {
                        let changed: Vec<f64> = plans
                            .iter()
                            .map(|p| p.monthly_usd * if p.provider == "aeza" { factor } else { 1. })
                            .collect();
                        let now_aeza = choose(&s, &b, &plans, &changed, sensitivity)
                            .is_some_and(|i| plans[i].provider == "aeza");
                        if factor > 1. {
                            assert!(
                                originally_aeza || !now_aeza,
                                "price rise attracted buyer {id} with unchanged incumbent {incumbent:?}"
                            );
                        } else {
                            assert!(
                                !originally_aeza || now_aeza,
                                "price cut repelled buyer {id} with unchanged incumbent {incumbent:?}"
                            );
                        }
                    }
                }
            }
        }
    }
}
