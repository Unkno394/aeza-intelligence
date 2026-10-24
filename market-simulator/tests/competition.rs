use aeza_market_simulator::{
    competition::{Request, analyze, region_matches},
    *,
};
fn fixture() -> Scenario {
    let mut s = example();
    s.seed = 42;
    s.months = 6;
    s.buyers_per_month = 40;
    s.arrival_weights = vec![1., 0., 0., 0., 0., 0.];
    s.aeza_plans = vec![s.aeza_plans[1].clone()];
    s.aeza_plans[0].monthly_usd = 24.;
    s.aeza_plans[0].regions = vec!["Stavanger".into()];
    s.segments = vec![Segment {
        name: "controlled_real_market".into(),
        weight: 1.,
        budget_min: 45.,
        budget_max: 55.,
        vcpu: 2.,
        ram_gb: 4.,
        disk_gb: 60.,
        region: Some("Stavanger".into()),
    }];
    s.monthly_departure_probability = 0.;
    s.preference_strength = 0.;
    s.switching_cost_utility = 0.05;
    s.outside_option_utility = -10.;
    s.strategies = vec![Strategy {
        name: "baseline".into(),
        aeza_price_multipliers: vec![1.; 6],
    }];
    s.market_events.clear();
    s
}
fn event(month: usize, mult: f64, available: bool) -> MarketEvent {
    MarketEvent {
        plan_id: "index-318".into(),
        starts_month: month,
        price_multiplier: mult,
        available,
        strategy: None,
    }
}
fn check(r: &Report) {
    for run in &r.runs {
        let mut total_revenue = 0.;
        let mut total_profit = 0.;
        for (i, m) in run.months.iter().enumerate() {
            assert_eq!(m.active_leases + m.outside_buyers, m.cumulative_buyers);
            let mut shares = 0.;
            for (provider, p) in &m.providers {
                let prev = if i == 0 {
                    0
                } else {
                    run.months[i - 1].providers[provider].active_leases
                };
                assert_eq!(p.at_risk, prev);
                assert_eq!(p.active_leases, p.renewals + p.new_leases + p.switches_in);
                assert_eq!(prev, p.renewals + p.switches_out + p.churn_to_outside);
                assert_eq!(p.new_leases, p.first_leases + p.returning_leases);
                assert_eq!(
                    p.switches_out,
                    m.flows
                        .iter()
                        .filter(|f| &f.from == provider && f.to != "outside")
                        .map(|f| f.events)
                        .sum::<usize>()
                );
                assert_eq!(
                    p.switches_in,
                    m.flows
                        .iter()
                        .filter(|f| &f.to == provider && f.from != "outside")
                        .map(|f| f.events)
                        .sum::<usize>()
                );
                shares += p.market_share;
                if provider == "aeza" {
                    let cost = r.scenario.aeza_fixed_cost_usd
                        + r.scenario.aeza_variable_cost_usd * p.active_leases as f64;
                    assert!((p.cost_usd.unwrap() - cost).abs() < 1e-8);
                    assert!((p.profit_usd.unwrap() - (p.revenue_usd - cost)).abs() < 1e-8);
                    total_revenue += p.revenue_usd;
                    total_profit += p.profit_usd.unwrap();
                }
            }
            if m.active_leases > 0 {
                assert!((shares - 1.).abs() < 1e-10);
            }
            for d in &m.decisions {
                if d.reason == "renewal_utility_choice" {
                    assert!(
                        d.utility.unwrap() > d.previous_offer_utility.unwrap_or(f64::NEG_INFINITY)
                    );
                }
            }
        }
        assert!((run.aeza_total_revenue_usd - total_revenue).abs() < 1e-8);
        assert!((run.aeza_total_profit_usd - total_profit).abs() < 1e-8);
        for f in &run.flows {
            assert!(f.unique_buyers <= f.events);
            assert_eq!(
                f.events,
                run.months
                    .iter()
                    .flat_map(|m| &m.flows)
                    .filter(|x| x.from == f.from && x.to == f.to)
                    .map(|x| x.events)
                    .sum::<usize>()
            );
        }
    }
}
#[test]
fn genuine_renewal_switches_on_observed_catalog_and_recovery() {
    let base = fixture();
    let baseline = simulate(base.clone(), catalog()).unwrap();
    check(&baseline);
    for r in &baseline.runs {
        assert_eq!(r.months[0].providers["aeza"].active_leases, 40);
        assert!(
            r.months
                .iter()
                .all(|m| m.providers["aeza"].switches_out == 0)
        );
    }
    for case in ["own_rise", "competitor_cut", "entry", "recovery"] {
        let mut s = base.clone();
        match case {
            "own_rise" => s.strategies[0].aeza_price_multipliers = vec![1., 1., 1.3, 1.3, 1.3, 1.3],
            "competitor_cut" => s.market_events = vec![event(3, 0.7, true)],
            "entry" => {
                s.aeza_plans[0].monthly_usd = 30.5;
                s.market_events = vec![event(1, 1., false), event(3, 1., true)];
            }
            _ => s.strategies[0].aeza_price_multipliers = vec![1., 1., 1.3, 1.3, 0.7, 0.7],
        }
        let r = simulate(s.clone(), catalog()).unwrap();
        check(&r);
        assert_eq!(
            serde_json::to_vec(&r).unwrap(),
            serde_json::to_vec(&simulate(s, catalog()).unwrap()).unwrap()
        );
        for run in &r.runs {
            assert_eq!(run.months[1].providers["aeza"].active_leases, 40, "{case}");
            let expected = if case == "entry" && run.sensitivity == Sensitivity::Low {
                37
            } else {
                40
            };
            assert_eq!(
                run.months[2].providers["aeza"].switches_out, expected,
                "{case} {:?}",
                run.sensitivity
            );
            assert_eq!(run.months[2].providers["UpCloud"].switches_in, expected);
            if case == "recovery" {
                assert_eq!(run.months[4].providers["aeza"].switches_in, 40);
                assert_eq!(run.months[4].providers["UpCloud"].switches_out, 40);
            }
            if case == "competitor_cut" {
                assert!((run.months[2].effective_prices_usd["index-318"] - 17.563).abs() < 1e-10);
            }
            if case == "entry" {
                assert!(!run.months[0].effective_prices_usd.contains_key("index-318"));
                assert_eq!(run.months[2].effective_prices_usd["index-318"], 25.09);
            }
        }
    }
}
#[test]
fn switching_cost_can_prevent_migration_without_forcing_it() {
    let mut s = fixture();
    s.switching_cost_utility = 20.;
    s.strategies[0].aeza_price_multipliers = vec![1., 1., 1.3, 1.3, 1.3, 1.3];
    let r = simulate(s, catalog()).unwrap();
    check(&r);
    assert!(r.runs.iter().all(|r| {
        r.months
            .iter()
            .all(|m| m.providers["aeza"].switches_out == 0)
    }));
}
#[test]
fn provenance_ranking_price_math_and_region_rules() {
    assert!(region_matches(&["EU".into()], "Frankfurt 2"));
    assert!(region_matches(&["Paris2".into()], "EU"));
    assert!(!region_matches(&[], "EU"));
    assert!(!region_matches(&["London".into()], "EU"));
    assert!(!region_matches(&["EU".into()], "Stavanger"));
    let mut s = fixture();
    s.market_events = vec![event(3, 0.7, true)];
    let a = analyze(
        Request {
            scenario: s,
            strategy: "baseline".into(),
            month: 3,
            region: Some("Stavanger".into()),
            verified_only: false,
        },
        catalog(),
    )
    .unwrap();
    let row = a
        .comparisons
        .iter()
        .find(|x| x.competitor.id == "index-318")
        .unwrap();
    assert_eq!(row.competitor.monthly_usd, 25.09);
    assert!(row.competitor.source.starts_with("https://"));
    assert_eq!(row.rank, Some(1));
    assert_eq!(row.resource_match, "exact_specs");
    assert!((row.difference_percent.unwrap() - (17.563 / 24. - 1.) * 100.).abs() < 1e-10);
    assert!(row.scenario_price_changed);
    assert!(
        a.comparisons
            .iter()
            .filter(|x| x.competitor.regions.is_empty())
            .all(|x| !x.comparable)
    );
}
#[test]
fn invalid_events_and_empty_markets() {
    let mut s = fixture();
    s.market_events = vec![event(0, 0.7, true)];
    assert!(simulate(s.clone(), catalog()).is_err());
    s.market_events[0].starts_month = 3;
    s.market_events[0].plan_id = "fabricated-price".into();
    assert!(simulate(s.clone(), catalog()).is_err());
    s.market_events.clear();
    s.buyers_per_month = 0;
    check(&simulate(s.clone(), catalog()).unwrap());
    s.buyers_per_month = 40;
    s.segments[0].disk_gb = 1e8;
    let r = simulate(s, catalog()).unwrap();
    check(&r);
    assert!(
        r.runs
            .iter()
            .all(|r| r.months.iter().all(|m| m.active_leases == 0))
    );
}

#[test]
fn repeated_departures_and_returns_are_events_not_unique_people() {
    let mut s = fixture();
    s.buyers_per_month = 1;
    s.monthly_departure_probability = 1.;
    let r = simulate(s, catalog()).unwrap();
    check(&r);
    for run in &r.runs {
        let out = run
            .flows
            .iter()
            .find(|f| f.from == "aeza" && f.to == "outside")
            .unwrap();
        let into = run
            .flows
            .iter()
            .find(|f| f.from == "outside" && f.to == "aeza")
            .unwrap();
        assert_eq!((out.events, out.unique_buyers), (3, 1));
        assert_eq!((into.events, into.unique_buyers), (3, 1));
        assert_eq!(
            run.months
                .iter()
                .map(|m| m.providers["aeza"].first_leases)
                .sum::<usize>(),
            1
        );
        assert_eq!(
            run.months
                .iter()
                .map(|m| m.providers["aeza"].returning_leases)
                .sum::<usize>(),
            2
        );
    }
}
#[test]
fn strategy_scoped_competitor_events_leave_control_identical() {
    let mut s = fixture();
    let control = simulate(s.clone(), catalog()).unwrap();
    s.strategies.push(Strategy {
        name: "cut_only".into(),
        aeza_price_multipliers: vec![1.; 6],
    });
    let mut e = event(3, 0.7, true);
    e.strategy = Some("cut_only".into());
    s.market_events = vec![e];
    let r = simulate(s, catalog()).unwrap();
    check(&r);
    for sensitivity in [Sensitivity::Low, Sensitivity::Medium, Sensitivity::High] {
        let base = r
            .runs
            .iter()
            .find(|r| r.strategy == "baseline" && r.sensitivity == sensitivity)
            .unwrap();
        let old = control
            .runs
            .iter()
            .find(|r| r.sensitivity == sensitivity)
            .unwrap();
        assert_eq!(
            serde_json::to_value(base).unwrap(),
            serde_json::to_value(old).unwrap()
        );
        let cut = r
            .runs
            .iter()
            .find(|r| r.strategy == "cut_only" && r.sensitivity == sensitivity)
            .unwrap();
        assert_eq!(cut.months[2].providers["aeza"].switches_out, 40);
    }
}
