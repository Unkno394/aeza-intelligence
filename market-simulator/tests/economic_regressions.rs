use aeza_market_simulator::*;
#[test]
fn zero_buyers_is_a_valid_empty_market_with_fixed_expenses() {
    let mut s = example();
    s.buyers_per_month = 0;
    let r = simulate(s, catalog()).expect("an empty market is a valid economic scenario");
    for run in r.runs {
        for m in run.months {
            assert_eq!(m.cumulative_buyers, 0);
            assert_eq!(m.active_leases, 0);
            for p in m.providers.values() {
                assert_eq!(p.revenue_usd, 0.);
                assert_eq!(p.market_share, 0.);
            }
            assert_eq!(m.providers["aeza"].profit_usd, Some(-100.));
        }
    }
}
#[test]
fn exact_utility_ties_follow_documented_global_plan_id_order() {
    let mut s = example();
    s.months = 1;
    s.arrival_weights = vec![1.];
    s.buyers_per_month = 10;
    s.monthly_departure_probability = 0.;
    s.preference_strength = 0.;
    s.switching_cost_utility = 0.;
    s.sensitivities = vec![Sensitivity::Low];
    s.strategies = vec![Strategy {
        name: "tie".into(),
        aeza_price_multipliers: vec![1.],
    }];
    s.segments.truncate(1);
    s.segments[0].budget_min = 30.;
    s.segments[0].budget_max = 30.;
    s.aeza_plans.truncate(1);
    s.aeza_plans[0].id = "aaa-aeza".into();
    s.aeza_plans[0].monthly_usd = 5.;
    let mut p = s.aeza_plans[0].clone();
    p.id = "zzz-competitor".into();
    p.provider = "competitor".into();
    let r = simulate(s, vec![p]).unwrap();
    assert_eq!(
        r.runs[0].months[0].providers["aeza"].active_leases, 10,
        "equal utility must not always privilege competitors over Aéza"
    );
}
#[test]
fn financial_values_and_market_shares_preserve_bits_across_json() {
    for value in [
        195.95999999999998_f64,
        0.12352941176470589,
        0.36470588235294116,
    ] {
        let encoded = serde_json::to_string(&value).unwrap();
        let decoded: f64 = serde_json::from_str(&encoded).unwrap();
        assert_eq!(
            value.to_bits(),
            decoded.to_bits(),
            "JSON must preserve financial/report values: {encoded}"
        );
    }
}
