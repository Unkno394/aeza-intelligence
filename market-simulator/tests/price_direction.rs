use aeza_market_simulator::*;
fn scenario(multiplier: f64) -> Scenario {
    let mut s = example();
    s.seed = 42;
    s.strategies = vec![Strategy {
        name: "custom".into(),
        aeza_price_multipliers: vec![multiplier; s.months],
    }];
    s
}
#[test]
fn exact_15_percent_regression_and_actual_engine_prices() {
    for (multiplier, prices, leases, revenue, profit) in [
        (1., [5., 10., 20.], 146, 6225., 3966.),
        (0.15, [0.75, 1.5, 3.], 506, 2257.5, -3688.5),
        (1.15, [5.75, 11.5, 23.], 103, 5474., 3647.),
        (0.85, [4.25, 8.5, 17.], 200, 6625.75, 3778.75),
    ] {
        let s = scenario(multiplier);
        let plans = s.aeza_plans.clone();
        let r = simulate(s, catalog()).unwrap();
        let run = r
            .runs
            .iter()
            .find(|r| r.sensitivity == Sensitivity::Medium)
            .unwrap();
        assert_eq!(
            run.months.last().unwrap().providers["aeza"].active_leases,
            leases
        );
        assert!((run.aeza_total_revenue_usd - revenue).abs() < 1e-8);
        assert!((run.aeza_total_profit_usd - profit).abs() < 1e-8);
        for month in &run.months {
            for (i, p) in plans.iter().enumerate() {
                assert!((month.aeza_effective_prices_usd[&p.id] - prices[i]).abs() < 1e-10);
            }
        }
    }
}
#[test]
fn raising_own_prices_cannot_raise_first_cohort_demand() {
    for sensitivity in [Sensitivity::Low, Sensitivity::Medium, Sensitivity::High] {
        let mut prior = usize::MAX;
        for multiplier in [0.6, 0.8, 0.85, 0.9, 1., 1.1, 1.15, 1.2, 1.4] {
            let mut s = scenario(multiplier);
            s.months = 1;
            s.arrival_weights = vec![1.];
            s.strategies[0].aeza_price_multipliers = vec![multiplier];
            s.sensitivities = vec![sensitivity];
            let r = simulate(s, catalog()).unwrap();
            let count = r.runs[0].months[0].providers["aeza"].active_leases;
            assert!(
                count <= prior,
                "Higher own price made the identical first cohort more willing to rent: {sensitivity:?}, {multiplier}, {count}>{prior}"
            );
            prior = count;
        }
    }
}
