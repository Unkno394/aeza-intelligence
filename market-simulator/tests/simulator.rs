use aeza_market_simulator::*;
fn fixture() -> (Scenario, Vec<Plan>) {
    let mut s = example();
    s.months = 3;
    s.buyers_per_month = 40;
    s.arrival_weights = vec![1.; 3];
    s.strategies = vec![Strategy {
        name: "baseline".into(),
        aeza_price_multipliers: vec![1.; 3],
    }];
    s.sensitivities = vec![Sensitivity::Medium];
    s.monthly_departure_probability = 0.;
    s.preference_strength = 0.;
    s.switching_cost_utility = 0.;
    s.aeza_plans.truncate(1);
    s.aeza_plans[0].monthly_usd = 5.;
    s.aeza_plans[0].vcpu = 8.;
    s.aeza_plans[0].ram_gb = 16.;
    s.aeza_plans[0].disk_gb = 200.;
    s.segments.truncate(1);
    s.segments[0].budget_min = 20.;
    s.segments[0].budget_max = 20.;
    let mut p = s.aeza_plans[0].clone();
    p.id = "competitor".into();
    p.provider = "competitor".into();
    p.monthly_usd = 10.;
    (s, vec![p])
}
#[test]
fn replay_is_byte_identical() {
    let (s, c) = fixture();
    assert_eq!(
        serde_json::to_vec(&simulate(s.clone(), c.clone()).unwrap()).unwrap(),
        serde_json::to_vec(&simulate(s, c).unwrap()).unwrap()
    )
}
#[test]
fn accounting_and_conservation() {
    let (s, c) = fixture();
    let r = simulate(s.clone(), c).unwrap();
    for (i, m) in r.runs[0].months.iter().enumerate() {
        let a = &m.providers["aeza"];
        assert_eq!(a.active_leases, 40 * (i + 1));
        assert_eq!(a.new_leases, 40);
        assert_eq!(a.renewals, 40 * i);
        assert_eq!(a.revenue_usd, a.active_leases as f64 * 5.);
        assert_eq!(
            a.profit_usd.unwrap(),
            a.revenue_usd - a.active_leases as f64 * 3. - 100.
        );
        assert_eq!(m.active_leases + m.outside_buyers, m.cumulative_buyers);
        assert!((m.providers.values().map(|p| p.market_share).sum::<f64>() - 1.).abs() < 1e-12);
        assert_eq!(m.providers["competitor"].profit_usd, None)
    }
}
#[test]
fn price_change_causes_switches_at_renewal() {
    let (mut s, c) = fixture();
    s.strategies[0].aeza_price_multipliers = vec![1., 3., 3.];
    let r = simulate(s, c).unwrap();
    let m = &r.runs[0].months[1];
    assert_eq!(m.providers["aeza"].switches_out, 40);
    assert_eq!(m.providers["competitor"].switches_in, 40);
    assert_eq!(m.providers["competitor"].active_leases, 80)
}
#[test]
fn departure_is_not_same_period_repurchase() {
    let (mut s, c) = fixture();
    s.monthly_departure_probability = 1.;
    let r = simulate(s, c).unwrap();
    assert_eq!(r.runs[0].months[1].providers["aeza"].churn_to_outside, 40);
    assert_eq!(r.runs[0].months[1].outside_buyers, 40)
}
#[test]
fn budget_excludes_all_plans() {
    let (mut s, c) = fixture();
    s.segments[0].budget_min = 1.;
    s.segments[0].budget_max = 1.;
    let r = simulate(s, c).unwrap();
    assert!(
        r.runs[0]
            .months
            .iter()
            .all(|m| m.active_leases == 0 && m.providers["aeza"].profit_usd == Some(-100.))
    )
}
#[test]
fn resource_requirements_are_hard_constraints() {
    let (mut s, c) = fixture();
    s.segments[0].ram_gb = 100.;
    assert_eq!(simulate(s, c).unwrap().runs[0].months[0].active_leases, 0)
}
#[test]
fn unknown_region_is_not_a_match() {
    let (mut s, mut c) = fixture();
    s.segments[0].region = Some("US".into());
    c[0].regions = vec![];
    assert_eq!(simulate(s, c).unwrap().runs[0].months[0].active_leases, 0)
}
#[test]
fn high_sensitivity_can_choose_outside() {
    let (mut s, c) = fixture();
    s.aeza_plans[0].monthly_usd = 15.;
    s.segments[0].budget_min = 10.;
    s.segments[0].budget_max = 10.;
    s.sensitivities = vec![Sensitivity::Low, Sensitivity::Medium, Sensitivity::High];
    let r = simulate(s, c).unwrap();
    assert_eq!(r.runs[0].months[0].active_leases, 40);
    assert_eq!(r.runs[2].months[0].active_leases, 0)
}
#[test]
fn no_price_change_strategies_match() {
    let (mut s, c) = fixture();
    let mut second = s.strategies[0].clone();
    second.name = "copy".into();
    s.strategies.push(second);
    let r = simulate(s, c).unwrap();
    assert_eq!(
        serde_json::to_value(&r.runs[0].months).unwrap(),
        serde_json::to_value(&r.runs[1].months).unwrap()
    )
}
#[test]
fn external_probabilities_and_provenance() {
    let (mut s, c) = fixture();
    s.external_models = vec![ModelOutput {
        slot: "snapshot".into(),
        model_version: "external-1".into(),
        source: "test://fixture".into(),
        schema_version: 1,
        semantics: "calibrated_monthly_departure_probability".into(),
        monthly_departure_probability: vec![1.; 3],
    }];
    let r = simulate(s.clone(), c.clone()).unwrap();
    assert_eq!(r.runs[0].months[1].outside_buyers, 40);
    s.external_models[0].semantics = "risk_score".into();
    assert!(simulate(s, c).is_err())
}
#[test]
fn invalid_inputs_are_errors() {
    let (s, c) = fixture();
    let mut zero_price = s.clone();
    zero_price.aeza_plans[0].monthly_usd = 0.0;
    assert!(simulate(zero_price, c.clone()).is_err());
    let mut bad = s.clone();
    bad.aeza_plans[0].monthly_usd = f64::NAN;
    assert!(simulate(bad, c.clone()).is_err());
    let mut bad = s.clone();
    bad.months = 25;
    assert!(simulate(bad, c.clone()).is_err());
    let mut bad = s.clone();
    bad.strategies[0].aeza_price_multipliers = vec![];
    assert!(simulate(bad, c.clone()).is_err());
    let mut bad = s.clone();
    bad.monthly_departure_probability = 1.1;
    assert!(simulate(bad, c.clone()).is_err());
    let mut bad = c.clone();
    bad.push(c[0].clone());
    assert!(simulate(s, bad).is_err())
}
#[test]
fn catalog_order_does_not_change_run() {
    let (s, mut c) = fixture();
    let mut p = c[0].clone();
    p.id = "other".into();
    p.provider = "other".into();
    c.push(p);
    let a = simulate(s.clone(), c.clone()).unwrap();
    c.reverse();
    let b = simulate(s, c).unwrap();
    assert_eq!(a.replay_sha256, b.replay_sha256);
    assert_eq!(
        serde_json::to_value(a.runs).unwrap(),
        serde_json::to_value(b.runs).unwrap()
    )
}
#[test]
fn bundled_data_quality_and_example() {
    let c = catalog();
    assert_eq!(c.len(), 322);
    assert!(c.iter().all(|p| p.origin == "observed_list_price"));
    example().validate(&c).unwrap();
    let q: serde_json::Value =
        serde_json::from_str(include_str!("../data/akash_quality.json")).unwrap();
    assert_eq!(q["with_start_utc"], 0);
    assert_eq!(q["rows"], 627346)
}
#[test]
fn lifecycle_stock_flow_identity() {
    let mut s = example();
    s.buyers_per_month = 20;
    let r = simulate(s, catalog()).unwrap();
    for run in r.runs {
        let mut prior = std::collections::BTreeMap::<String, usize>::new();
        for m in run.months {
            for (p, k) in m.providers {
                assert_eq!(
                    k.active_leases,
                    prior.get(&p).copied().unwrap_or(0) + k.new_leases + k.switches_in
                        - k.switches_out
                        - k.churn_to_outside
                );
                prior.insert(p, k.active_leases);
            }
        }
    }
}
