use aeza_market_simulator::*;
use serde_json::{Value, json};
use std::collections::BTreeMap;
#[derive(Clone)]
pub struct Case {
    pub id: String,
    pub expected: String,
    pub scenario: Scenario,
    pub market: Vec<Plan>,
    pub aeza_active: Option<Vec<usize>>,
    pub outside: Option<Vec<usize>>,
    pub switches: Option<Vec<usize>>,
}
pub fn fixture() -> (Scenario, Vec<Plan>) {
    let mut s = example();
    s.seed = 42;
    s.months = 3;
    s.buyers_per_month = 20;
    s.arrival_weights = vec![1.; 3];
    s.monthly_departure_probability = 0.;
    s.preference_strength = 0.;
    s.switching_cost_utility = 0.;
    s.outside_option_utility = 0.;
    s.external_models.clear();
    s.sensitivities = vec![Sensitivity::Medium];
    s.strategies = vec![Strategy {
        name: "test".into(),
        aeza_price_multipliers: vec![1.; 3],
    }];
    s.segments.truncate(1);
    let g = &mut s.segments[0];
    g.budget_min = 30.;
    g.budget_max = 30.;
    g.vcpu = 1.;
    g.ram_gb = 1.;
    g.disk_gb = 20.;
    g.region = None;
    s.aeza_plans.truncate(1);
    let p = &mut s.aeza_plans[0];
    p.id = "aaa-aeza".into();
    p.vcpu = 1.;
    p.ram_gb = 1.;
    p.disk_gb = 20.;
    p.monthly_usd = 10.;
    let mut c = p.clone();
    c.id = "zzz-competitor".into();
    c.provider = "competitor".into();
    c.monthly_usd = 11.5;
    (s, vec![c])
}
fn case(id: &str, expected: &str, s: Scenario, c: Vec<Plan>, counts: Option<Vec<usize>>) -> Case {
    Case {
        id: id.into(),
        expected: expected.into(),
        scenario: s,
        market: c,
        aeza_active: counts,
        outside: None,
        switches: None,
    }
}
pub fn cases() -> Vec<Case> {
    let mut out = vec![];
    for (label, level) in [
        ("low", Sensitivity::Low),
        ("medium", Sensitivity::Medium),
        ("high", Sensitivity::High),
    ] {
        for (change, mul) in [
            ("minus40", 0.6),
            ("minus20", 0.8),
            ("minus10", 0.9),
            ("baseline", 1.),
            ("plus10", 1.1),
            ("plus20", 1.2),
            ("plus40", 1.4),
        ] {
            let (mut s, c) = fixture();
            s.sensitivities = vec![level];
            s.strategies[0].aeza_price_multipliers = vec![mul; 3];
            let counts = if mul <= 1.1 {
                vec![20, 40, 60]
            } else {
                vec![0; 3]
            };
            out.push(case(&format!("price_{label}_{change}"),&format!("Fixed budget 30, competitor 11.50: cheaper affordable provider wins. Aéza price {:.2}. No claim of increasing profit under discounts.",10.*mul),s,c,Some(counts)));
            let mut s = example();
            s.seed = 42;
            s.buyers_per_month = 30;
            s.sensitivities = vec![level];
            s.strategies = vec![Strategy {
                name: change.into(),
                aeza_price_multipliers: vec![mul; s.months],
            }];
            out.push(case(&format!("real_{label}_{change}"),"Observed catalog: conservation and financial arithmetic; first-period own demand is nonincreasing with own price. No required long-run profit ordering.",s,catalog(),None));
        }
    }
    for n in [0, 10, 20, 40] {
        let (mut s, c) = fixture();
        s.buyers_per_month = n;
        out.push(case(&format!("population_{n}"),"With cheaper Aéza, no churn or capacity limit: active equals cumulative arrivals; revenue is active times price; fixed expenses survive empty market.",s,c,Some(vec![n,n*2,n*3])));
    }
    for (id, w, counts) in [
        ("arrivals_growing", vec![0.5, 1., 2.], vec![10, 30, 70]),
        ("arrivals_declining", vec![2., 1., 0.5], vec![40, 60, 70]),
        ("arrivals_stopped", vec![1., 0., 0.], vec![20, 20, 20]),
        ("arrival_weights_zero", vec![0.; 3], vec![0; 3]),
    ] {
        let (mut s, c) = fixture();
        s.arrival_weights = w;
        out.push(case(
            id,
            "Arrival flow may decline without reducing the existing stock when churn is zero.",
            s,
            c,
            Some(counts),
        ));
    }
    for (name, p, counts) in [
        ("zero", 0., Some(vec![20, 40, 60])),
        ("25", 0.25, None),
        ("75", 0.75, None),
        ("100", 1., Some(vec![20, 20, 40])),
    ] {
        let (mut s, c) = fixture();
        s.monthly_departure_probability = p;
        out.push(case(&format!("churn_{name}"),"All first-cohort buyers start before any renewal churn. At 100% departure the path is 20/20/40 because outside buyers can reactivate next period. Compare nested churn only at month 2.",s,c,counts));
    }
    for (id, cost, counts, switches) in [
        ("switching_free", 0., vec![20, 0, 0], vec![0, 20, 0]),
        ("switching_costly", 1., vec![20, 20, 20], vec![0, 0, 0]),
    ] {
        let (mut s, mut c) = fixture();
        c[0].monthly_usd = 12.;
        s.strategies[0].aeza_price_multipliers = vec![1., 2., 2.];
        s.switching_cost_utility = cost;
        let mut x = case(
            id,
            "Incumbents compare Aéza 20 vs competitor 12; utility gain is 0.533. A switching penalty of 1 retains incumbents; new buyers still choose the competitor.",
            s,
            c,
            Some(counts),
        );
        x.switches = Some(switches);
        out.push(x);
    }
    let (mut s, mut c) = fixture();
    c[0].monthly_usd = 12.;
    s.strategies[0].aeza_price_multipliers = vec![1., 4., 4.];
    s.switching_cost_utility = 1.;
    let mut x = case(
        "budget_overrides_loyalty",
        "Current plan price 40 exceeds budget 30. Switching friction must not permit an unaffordable renewal.",
        s,
        c,
        Some(vec![20, 0, 0]),
    );
    x.switches = Some(vec![0, 20, 0]);
    out.push(x);
    for (id, v, f) in [
        ("cost_base", 3., 100.),
        ("cost_variable", 7., 100.),
        ("cost_fixed", 3., 200.),
        ("cost_zero", 0., 0.),
        ("cost_loss", 60., 1000.),
    ] {
        let (mut s, c) = fixture();
        s.aeza_variable_cost_usd = v;
        s.aeza_fixed_cost_usd = f;
        out.push(case(id,"Costs must not alter customer choice. Monthly profit = active*(price-variable)-fixed, with negative profit allowed.",s,c,Some(vec![20,40,60])));
    }
    for (id, price, counts) in [
        ("competitor_cheaper", 8., vec![0; 3]),
        ("competitor_dearer", 14., vec![20, 40, 60]),
        ("competitor_equal", 10., vec![20, 40, 60]),
    ] {
        let (s, mut c) = fixture();
        c[0].monthly_usd = price;
        out.push(case(id,"Identical resources and fixed budgets: cheaper offer wins. Exact utility ties use global plan ID order, not a privileged provider class.",s,c,Some(counts)));
    }
    for (label, level, counts) in [
        ("low", Sensitivity::Low, vec![20, 40, 60]),
        ("medium", Sensitivity::Medium, vec![20, 40, 60]),
        ("high", Sensitivity::High, vec![0; 3]),
    ] {
        let (mut s, mut c) = fixture();
        s.sensitivities = vec![level];
        s.segments[0].budget_min = 20.;
        s.segments[0].budget_max = 20.;
        c[0].monthly_usd = 25.;
        out.push(case(&format!("outside_{label}"),"At price/budget=0.5 and zero preference, utility is positive for low/medium but negative for high sensitivity, so high chooses outside.",s,c,Some(counts)));
    }
    let (mut s, c) = fixture();
    s.segments[0].ram_gb = 100.;
    let mut x = case(
        "no_matching_resources",
        "No plan meets RAM requirement: all buyers outside, zero revenue, loss equals fixed expense.",
        s,
        c,
        Some(vec![0; 3]),
    );
    x.outside = Some(vec![20, 40, 60]);
    out.push(x);
    let (mut s, c) = fixture();
    s.segments[0].budget_min = 1.;
    s.segments[0].budget_max = 1.;
    let mut x = case(
        "no_matching_budget",
        "Every plan exceeds budget: no leases or revenue, fixed expenses continue.",
        s,
        c,
        Some(vec![0; 3]),
    );
    x.outside = Some(vec![20, 40, 60]);
    out.push(x);
    let (mut s, mut c) = fixture();
    s.segments[0].region = Some("missing-region".into());
    c[0].regions.clear();
    let mut x = case(
        "no_matching_region",
        "Unknown locations cannot satisfy an explicit region requirement.",
        s,
        c,
        Some(vec![0; 3]),
    );
    x.outside = Some(vec![20, 40, 60]);
    out.push(x);
    let (mut s, mut c) = fixture();
    s.segments[0].budget_min = 20.;
    s.segments[0].budget_max = 20.;
    s.aeza_plans[0].monthly_usd = 20.;
    c[0].monthly_usd = 21.;
    s.sensitivities = vec![Sensitivity::Low];
    out.push(case(
        "exact_budget_allowed",
        "Price equal to budget is affordable when utility exceeds outside.",
        s,
        c,
        Some(vec![20, 40, 60]),
    ));
    let (mut s, mut c) = fixture();
    s.sensitivities = vec![Sensitivity::High];
    s.segments[0].budget_min = 25.;
    s.segments[0].budget_max = 25.;
    c[0].monthly_usd = 20.;
    out.push(case(
        "outside_exact_tie",
        "Utility exactly equals outside at 2 - 5*10/25 = 0; strict acceptance means no purchase.",
        s,
        c,
        Some(vec![0; 3]),
    ));
    let (mut s, c) = fixture();
    s.strategies.push(Strategy {
        name: "renamed-same-price".into(),
        aeza_price_multipliers: vec![1.; 3],
    });
    out.push(case(
        "identical_strategies",
        "Labels cannot change economic trajectories when prices and seed are identical.",
        s,
        c,
        Some(vec![20, 40, 60]),
    ));
    let (mut s, mut c) = fixture();
    let mut large = s.aeza_plans[0].clone();
    large.id = "bbb-large".into();
    large.name = "Controlled large".into();
    large.monthly_usd = 20.;
    large.vcpu = 4.;
    large.ram_gb = 8.;
    large.disk_gb = 80.;
    s.aeza_plans.push(large.clone());
    s.segments[0].vcpu = 4.;
    s.segments[0].ram_gb = 8.;
    s.segments[0].disk_gb = 80.;
    c[0].vcpu = 4.;
    c[0].ram_gb = 8.;
    c[0].disk_gb = 80.;
    c[0].monthly_usd = 21.;
    out.push(case("tiered_resource_billing", "The cheap Aéza plan fails resource requirements. Only the large Aéza plan at 20 is selected, so revenue must be 400/800/1200, not count times cheapest advertised price.", s, c, Some(vec![20,40,60])));
    out
}
fn equal(errors: &mut Vec<String>, condition: bool, message: String) {
    if !condition {
        errors.push(message)
    }
}
fn close(errors: &mut Vec<String>, a: f64, b: f64, message: String) {
    equal(
        errors,
        (a - b).abs() <= 1e-7_f64.max(b.abs() * 1e-10),
        format!("{message}: {a} != {b}"),
    )
}
pub fn verify(case: &Case, r: &Report) -> Vec<String> {
    let mut errors = vec![];
    equal(
        &mut errors,
        r.scenario.seed == 42,
        "fixed seed not echoed".into(),
    );
    equal(
        &mut errors,
        r.runs.len() == case.scenario.strategies.len() * case.scenario.sensitivities.len(),
        "strategy/sensitivity product".into(),
    );
    for run in &r.runs {
        let schedule = &r
            .scenario
            .strategies
            .iter()
            .find(|s| s.name == run.strategy)
            .unwrap()
            .aeza_price_multipliers;
        let mut prior = BTreeMap::<String, usize>::new();
        let mut total_buyers = 0;
        let mut total_revenue = 0.;
        let mut total_profit = 0.;
        for (i, m) in run.months.iter().enumerate() {
            let expected_arrivals = (r.scenario.buyers_per_month as f64
                * r.scenario.arrival_weights[i])
                .round() as usize;
            total_buyers += expected_arrivals;
            equal(
                &mut errors,
                m.arrivals == expected_arrivals && m.cumulative_buyers == total_buyers,
                format!("month {} arrival/customer balance", i + 1),
            );
            let active: usize = m.providers.values().map(|p| p.active_leases).sum();
            equal(
                &mut errors,
                active == m.active_leases && active + m.outside_buyers == total_buyers,
                format!("month {} market stock/outside balance", i + 1),
            );
            let ins: usize = m.providers.values().map(|p| p.switches_in).sum();
            let outs: usize = m.providers.values().map(|p| p.switches_out).sum();
            equal(
                &mut errors,
                ins == outs,
                format!("month {} switches in/out balance", i + 1),
            );
            close(
                &mut errors,
                m.providers.values().map(|p| p.market_share).sum(),
                if active == 0 { 0. } else { 1. },
                format!("month {} share sum", i + 1),
            );
            for (provider, p) in &m.providers {
                let previous = prior.get(provider).copied().unwrap_or(0);
                equal(
                    &mut errors,
                    p.active_leases + p.switches_out + p.churn_to_outside
                        == previous + p.new_leases + p.switches_in,
                    format!("month {} {provider} stock-flow", i + 1),
                );
                equal(
                    &mut errors,
                    p.active_leases == p.new_leases + p.renewals + p.switches_in,
                    format!("month {} {provider} active composition", i + 1),
                );
                equal(
                    &mut errors,
                    previous == p.renewals + p.switches_out + p.churn_to_outside,
                    format!("month {} {provider} incumbent disposition", i + 1),
                );
                close(
                    &mut errors,
                    p.market_share,
                    if active == 0 {
                        0.
                    } else {
                        p.active_leases as f64 / active as f64
                    },
                    format!("month {} {provider} share", i + 1),
                );
                let mut prices: Vec<f64> = case
                    .market
                    .iter()
                    .chain(&case.scenario.aeza_plans)
                    .filter(|plan| &plan.provider == provider)
                    .map(|plan| {
                        plan.monthly_usd * if provider == "aeza" { schedule[i] } else { 1. }
                    })
                    .collect();
                prices.sort_by(f64::total_cmp);
                if p.active_leases == 0 {
                    close(
                        &mut errors,
                        p.revenue_usd,
                        0.,
                        "zero leases zero revenue".into(),
                    )
                } else {
                    equal(
                        &mut errors,
                        p.revenue_usd + 1e-7 >= prices[0] * p.active_leases as f64
                            && p.revenue_usd - 1e-7
                                <= prices[prices.len() - 1] * p.active_leases as f64,
                        format!("month {} {provider} tariff revenue bounds", i + 1),
                    );
                    if prices.first() == prices.last() {
                        close(
                            &mut errors,
                            p.revenue_usd,
                            prices[0] * p.active_leases as f64,
                            format!(
                                "month {} {provider} independent single-price billing",
                                i + 1
                            ),
                        );
                    }
                }
                if provider == "aeza" {
                    let cost = r.scenario.aeza_fixed_cost_usd
                        + r.scenario.aeza_variable_cost_usd * p.active_leases as f64;
                    close(
                        &mut errors,
                        p.cost_usd.unwrap(),
                        cost,
                        format!("month {} expenses", i + 1),
                    );
                    close(
                        &mut errors,
                        p.profit_usd.unwrap(),
                        p.revenue_usd - cost,
                        format!("month {} profit", i + 1),
                    );
                    total_revenue += p.revenue_usd;
                    total_profit += p.profit_usd.unwrap();
                } else {
                    equal(
                        &mut errors,
                        p.cost_usd.is_none() && p.profit_usd.is_none(),
                        "competitor unknown costs must not be fabricated".into(),
                    );
                }
                prior.insert(provider.clone(), p.active_leases);
            }
            if case.id == "tiered_resource_billing" {
                close(
                    &mut errors,
                    m.providers["aeza"].revenue_usd,
                    [400., 800., 1200.][i],
                    format!("month {} exact qualifying-tier billing", i + 1),
                );
            }
            if let Some(v) = &case.aeza_active {
                equal(
                    &mut errors,
                    m.providers["aeza"].active_leases == v[i],
                    format!(
                        "month {} oracle active: {} != {}",
                        i + 1,
                        m.providers["aeza"].active_leases,
                        v[i]
                    ),
                );
            }
            if let Some(v) = &case.outside {
                equal(
                    &mut errors,
                    m.outside_buyers == v[i],
                    format!("month {} oracle outside", i + 1),
                );
            }
            if let Some(v) = &case.switches {
                equal(
                    &mut errors,
                    m.providers["aeza"].switches_out == v[i],
                    format!("month {} oracle outgoing switches", i + 1),
                );
            }
        }
        close(
            &mut errors,
            run.aeza_total_revenue_usd,
            total_revenue,
            "cumulative revenue".into(),
        );
        close(
            &mut errors,
            run.aeza_total_profit_usd,
            total_profit,
            "cumulative profit".into(),
        );
    }
    errors
}
pub fn observation(r: &Report) -> Value {
    json!({"engine_version":r.engine_version,"replay_sha256":r.replay_sha256,"catalog_sha256":r.catalog_sha256,"scenario":r.scenario,"runs":r.runs.iter().map(|run|json!({"strategy":run.strategy,"sensitivity":run.sensitivity,"revenue":run.aeza_total_revenue_usd,"profit":run.aeza_total_profit_usd,"months":run.months})).collect::<Vec<_>>()})
}
pub fn relational(reports: &BTreeMap<String, Report>) -> Vec<Value> {
    let mut rows = vec![];
    for label in ["low", "medium", "high"] {
        let counts: Vec<usize> = [
            "minus40", "minus20", "minus10", "baseline", "plus10", "plus20", "plus40",
        ]
        .iter()
        .map(|change| {
            reports[&format!("real_{label}_{change}")].runs[0].months[0].providers["aeza"]
                .active_leases
        })
        .collect();
        let pass = counts.windows(2).all(|w| w[0] >= w[1]);
        rows.push(json!({"id":format!("relation_price_{label}"),"expected":"First-period Aéza demand cannot increase with own price, holding buyers, seed and all other utilities fixed. No assertion on long-run profit.","actual":{"first_month_active_by_increasing_price":counts},"passed":pass}));
    }
    let counts: Vec<usize> = ["zero", "25", "75", "100"]
        .iter()
        .map(|n| reports[&format!("churn_{n}")].runs[0].months[1].active_leases)
        .collect();
    rows.push(json!({"id":"relation_churn_month2","expected":"With the same initial incumbents, nested departure thresholds cannot increase second-month retention. Reactivation prevents extending this assumption to arbitrary later months.","actual":counts,"passed":counts.windows(2).all(|w|w[0]>=w[1])}));
    let base = &reports["cost_base"].runs[0];
    for id in ["cost_variable", "cost_fixed", "cost_zero", "cost_loss"] {
        let other = &reports[id].runs[0];
        let mut a = serde_json::to_value(&base.months).unwrap();
        let mut b = serde_json::to_value(&other.months).unwrap();
        for v in [&mut a, &mut b] {
            for m in v.as_array_mut().unwrap() {
                if let Some(ledger) = m["billing_ledger_usd"]["aeza"].as_object_mut() {
                    ledger.remove("cost");
                    ledger.remove("profit");
                }
                let p = &mut m["providers"]["aeza"];
                p.as_object_mut().unwrap().remove("cost_usd");
                p.as_object_mut().unwrap().remove("profit_usd");
            }
        }
        let s = &reports[id].scenario;
        let lease_periods: usize = base
            .months
            .iter()
            .map(|m| m.providers["aeza"].active_leases)
            .sum();
        let delta = -(s.aeza_variable_cost_usd - 3.) * lease_periods as f64
            - (s.aeza_fixed_cost_usd - 100.) * 3.;
        let actual = other.aeza_total_profit_usd - base.aeza_total_profit_usd;
        rows.push(json!({"id":format!("relation_{id}"),"expected":"Changing only costs leaves all customer flows, leases, revenue and market shares unchanged; profit delta equals negative incremental costs.","actual":{"demand_and_revenue_unchanged":a==b,"profit_delta":actual,"expected_profit_delta":delta},"passed":a==b&&(actual-delta).abs()<1e-7}));
    }
    let r = &reports["identical_strategies"].runs;
    rows.push(json!({"id":"relation_identical_strategies","expected":"Identical schedules and seed have identical full monthly economics.","actual":{"identical":serde_json::to_value(&r[0].months).unwrap()==serde_json::to_value(&r[1].months).unwrap()},"passed":serde_json::to_value(&r[0].months).unwrap()==serde_json::to_value(&r[1].months).unwrap()}));
    rows
}
pub fn save(name: &str, rows: &[Value]) {
    if let Ok(path) = std::env::var("MARKET_TEST_REPORT_DIR") {
        let p = std::path::Path::new(&path);
        std::fs::create_dir_all(p).unwrap();
        std::fs::write(p.join(name), serde_json::to_vec_pretty(rows).unwrap()).unwrap();
    }
}
