//! Seed uncertainty aggregation. Every point comes from the unchanged simulation engine.
use crate::{hash, money, simulate, Plan, Scenario, Sensitivity, VERSION};
use serde::{Deserialize, Serialize};
pub const SEED_COUNT: usize = 100;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub scenario: Scenario,
    pub sensitivity: Sensitivity,
}
#[derive(Clone, Debug, Serialize)]
pub struct Point {
    pub seed: u64,
    pub profit_rub: f64,
    pub exact_profit_rub: String,
    pub revenue_rub: f64,
    pub final_active: usize,
    pub loss_events: usize,
}
#[derive(Debug, Serialize)]
pub struct Distribution {
    pub min_rub: f64,
    pub max_rub: f64,
    pub median_rub: f64,
    pub p025_rub: f64,
    pub p25_rub: f64,
    pub p75_rub: f64,
    pub p975_rub: f64,
    pub sample_sd_rub: f64,
}
#[derive(Debug, Serialize)]
pub struct StrategyStatistics {
    pub strategy: String,
    pub profit: Distribution,
    pub loss_runs: usize,
    pub loss_probability_percent: f64,
    pub loss_probability_ci95_percent: [f64; 2],
    pub sole_wins: usize,
    pub tied_wins: usize,
    pub leading_percent: f64,
    pub leading_ci95_percent: [f64; 2],
    pub points: Vec<Point>,
}
#[derive(Debug, Serialize)]
pub struct Statistics {
    pub engine_version: String,
    pub currency: String,
    pub seeds: Vec<u64>,
    pub seed_count: usize,
    pub sensitivity: Sensitivity,
    pub scenario: Scenario,
    pub catalog_sha256: String,
    pub replay_sha256: String,
    pub tied_seed_count: usize,
    pub strategies: Vec<StrategyStatistics>,
    pub limitations: Vec<String>,
}
fn signed_money(v: &str) -> i128 {
    if let Some(v) = v.strip_prefix('-') {
        -money::decimal(v).expect("engine money")
    } else {
        money::decimal(v).expect("engine money")
    }
}
fn quantile(sorted: &[i128], n: usize, d: usize) -> i128 {
    let position = (sorted.len() - 1) * n;
    let lo = position / d;
    let r = position % d;
    if r == 0 {
        return sorted[lo];
    }
    let numerator = sorted[lo] * d as i128 + (sorted[lo + 1] - sorted[lo]) * r as i128;
    if numerator < 0 {
        -money::round(-numerator, d as i128).unwrap()
    } else {
        money::round(numerator, d as i128).unwrap()
    }
}
fn distribution(values: &[i128]) -> Distribution {
    let mut sorted = values.to_vec();
    sorted.sort();
    // Center in integer money before floating variance, avoiding cancellation at large totals.
    let origin = sorted[0];
    let shifted: Vec<f64> = values.iter().map(|v| money::float(v - origin)).collect();
    let center = shifted.iter().sum::<f64>() / values.len() as f64;
    let sd = (shifted.iter().map(|v| (v - center).powi(2)).sum::<f64>()
        / (values.len() - 1) as f64)
        .sqrt();
    Distribution {
        min_rub: money::float(sorted[0]),
        max_rub: money::float(*sorted.last().unwrap()),
        median_rub: money::float(quantile(&sorted, 1, 2)),
        p025_rub: money::float(quantile(&sorted, 1, 40)),
        p25_rub: money::float(quantile(&sorted, 1, 4)),
        p75_rub: money::float(quantile(&sorted, 3, 4)),
        p975_rub: money::float(quantile(&sorted, 39, 40)),
        sample_sd_rub: sd,
    }
}
pub fn wilson(k: usize, n: usize) -> [f64; 2] {
    let z = 1.959963984540054;
    let p = k as f64 / n as f64;
    let den = 1. + z * z / n as f64;
    let center = (p + z * z / (2. * n as f64)) / den;
    let half = z / den * (p * (1. - p) / n as f64 + z * z / (4. * (n * n) as f64)).sqrt();
    [
        if k == 0 { 0. } else { (center - half).max(0.) } * 100.,
        if k == n { 1. } else { (center + half).min(1.) } * 100.,
    ]
}
pub fn prepare(mut r: Request, market: &[Plan]) -> Result<Scenario, String> {
    if r.scenario.currency != "RUB" {
        return Err("statistics requires RUB".into());
    }
    r.scenario.seed = 0;
    r.scenario.sensitivities = vec![r.sensitivity];
    r.scenario.validate(market)?;
    let s = &r.scenario;
    let work = (s.months * (s.months + 1) / 2) as u128
        * s.buyers_per_month as u128
        * 2
        * (market.len() + s.aeza_plans.len()) as u128
        * s.strategies.len() as u128
        * SEED_COUNT as u128;
    if work > 750_000_000 {
        return Err("100-seed batch exceeds 750 million candidate evaluations; reduce buyers, months or strategies".into());
    }
    Ok(r.scenario)
}
pub fn analyze(s: Scenario, market: Vec<Plan>) -> Result<Statistics, String> {
    let sensitivity = *s.sensitivities.first().ok_or("one sensitivity required")?;
    let s = prepare(
        Request {
            scenario: s,
            sensitivity,
        },
        &market,
    )?;
    let mut points: Vec<Vec<Point>> = vec![vec![]; s.strategies.len()];
    let mut profits: Vec<Vec<i128>> = vec![vec![]; s.strategies.len()];
    let mut sole = vec![0; s.strategies.len()];
    let mut tied = sole.clone();
    let mut tied_seed_count = 0;
    let mut catalog_sha256 = String::new();
    for seed in 0..SEED_COUNT as u64 {
        let mut input = s.clone();
        input.seed = seed;
        let report = simulate(input, market.clone())?;
        if catalog_sha256.is_empty() {
            catalog_sha256 = report.catalog_sha256.clone();
        } else if catalog_sha256 != report.catalog_sha256 {
            return Err("catalog changed within seed batch".into());
        }
        let mut seed_profit = Vec::new();
        for (i, run) in report.runs.iter().enumerate() {
            let exact = &run.exact_totals_usd["profit"];
            let value = signed_money(exact);
            seed_profit.push(value);
            profits[i].push(value);
            points[i].push(Point {
                seed,
                profit_rub: money::float(value),
                exact_profit_rub: exact.clone(),
                revenue_rub: run.aeza_total_revenue_usd,
                final_active: run.months.last().unwrap().providers["aeza"].active_leases,
                loss_events: run
                    .months
                    .iter()
                    .map(|m| {
                        m.providers["aeza"].churn_to_outside + m.providers["aeza"].switches_out
                    })
                    .sum(),
            });
        }
        let best = *seed_profit.iter().max().unwrap();
        let winners: Vec<usize> = seed_profit
            .iter()
            .enumerate()
            .filter_map(|(i, v)| (*v == best).then_some(i))
            .collect();
        if winners.len() == 1 {
            sole[winners[0]] += 1;
        } else {
            tied_seed_count += 1;
            for i in winners {
                tied[i] += 1;
            }
        }
    }
    let strategies = s
        .strategies
        .iter()
        .enumerate()
        .map(|(i, strategy)| {
            let loss = profits[i].iter().filter(|v| **v < 0).count();
            let leading = sole[i] + tied[i];
            StrategyStatistics {
                strategy: strategy.name.clone(),
                profit: distribution(&profits[i]),
                loss_runs: loss,
                loss_probability_percent: loss as f64,
                loss_probability_ci95_percent: wilson(loss, SEED_COUNT),
                sole_wins: sole[i],
                tied_wins: tied[i],
                leading_percent: leading as f64,
                leading_ci95_percent: wilson(leading, SEED_COUNT),
                points: std::mem::take(&mut points[i]),
            }
        })
        .collect();
    let replay_sha256 = hash(
        &serde_json::to_vec(&(
            VERSION,
            "statistics-v1",
            &s,
            &catalog_sha256,
            (0..SEED_COUNT as u64).collect::<Vec<_>>(),
        ))
        .unwrap(),
    );
    Ok(Statistics{engine_version:VERSION.into(),currency:"RUB".into(),seeds:(0..SEED_COUNT as u64).collect(),seed_count:SEED_COUNT,sensitivity,scenario:s,catalog_sha256,replay_sha256,tied_seed_count,strategies,limitations:vec!["Seed 0–99: only simulated buyer and event variability; not market forecast uncertainty.".into(),"Ranges are empirical min/max and quantiles, not confidence intervals for future real profit. Mean profit is not returned.".into(),"Risk and leadership intervals use two-sided 95% Wilson score. 0/100 losses does not imply zero risk.".into(),"Leadership is relative to submitted strategies. Tied maxima are shown separately; top percentages can sum above 100% when tied.".into(),"Scenario inputs, FX, catalog, capacity and external model outputs are frozen throughout the batch.".into()]})
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn quantiles_sd_and_negative_profits() {
        let values: Vec<_> = (0..100)
            .map(|i| money::ticks(i as f64) - money::ticks(50.))
            .collect();
        let d = distribution(&values);
        assert_eq!(d.min_rub, -50.);
        assert_eq!(d.max_rub, 49.);
        assert_eq!(d.median_rub, -0.5);
        assert_eq!(d.p025_rub, -47.525);
        assert_eq!(d.p975_rub, 46.525);
        assert!((d.sample_sd_rub - (841.6666666666666f64).sqrt()).abs() < 1e-10);
        assert_eq!(quantile(&[1, 2], 1, 2), 2);
        assert_eq!(quantile(&[-2, -1], 1, 2), -2);
    }
    #[test]
    fn wilson_endpoints_and_known_risk() {
        let a = wilson(0, 100);
        assert_eq!(a[0], 0.);
        assert!((a[1] - 3.699349820698565).abs() < 1e-10);
        let b = wilson(100, 100);
        assert_eq!(b[1], 100.);
        let c = wilson(29, 100);
        assert!((c[0] - 21.014835749122274).abs() < 1e-10);
        assert!((c[1] - 38.53889117557111).abs() < 1e-10);
    }
    #[test]
    fn batch_is_reproducible_ties_are_not_sole_wins_and_no_profit_mean() {
        let mut s =
            crate::currency::scenario_to_rub(crate::example(), &crate::market_data::bundled_fx());
        s.buyers_per_month = 0;
        s.sensitivities = vec![Sensitivity::Medium];
        s.strategies.truncate(2);
        let a = analyze(s.clone(), crate::catalog()).unwrap();
        let b = analyze(s, crate::catalog()).unwrap();
        assert_eq!(
            serde_json::to_value(&a).unwrap(),
            serde_json::to_value(&b).unwrap()
        );
        assert_eq!(a.seed_count, 100);
        assert_eq!(a.tied_seed_count, 100);
        for x in &a.strategies {
            assert_eq!(x.sole_wins, 0);
            assert_eq!(x.tied_wins, 100);
            assert_eq!(x.loss_runs, 100);
            assert_eq!(x.profit.sample_sd_rub, 0.);
        }
        assert!(!serde_json::to_string(&a).unwrap().contains("mean_rub"));
    }
    #[test]
    fn zero_profit_is_not_a_loss() {
        let fx = crate::market_data::bundled_fx();
        let mut s = crate::currency::scenario_to_rub(crate::example(), &fx);
        s.buyers_per_month = 0;
        s.aeza_variable_cost_usd = 0.;
        s.aeza_fixed_cost_usd = 0.;
        s.sensitivities = vec![Sensitivity::Low];
        s.strategies.truncate(1);
        let market = crate::catalog()
            .into_iter()
            .take(1)
            .map(|p| crate::currency::plan_to_rub(p, &fx))
            .collect();
        let result = analyze(s, market).unwrap();
        let st = &result.strategies[0];
        assert_eq!(st.loss_runs, 0);
        assert_eq!(st.loss_probability_percent, 0.);
        assert_eq!(st.sole_wins, 100);
        assert_eq!(st.profit.min_rub, 0.);
        assert_eq!(st.profit.max_rub, 0.);
    }
    #[test]
    fn every_seed_point_matches_direct_engine() {
        let fx = crate::market_data::bundled_fx();
        let mut s = crate::currency::scenario_to_rub(crate::example(), &fx);
        s.months = 2;
        s.arrival_weights = vec![1., 1.];
        s.buyers_per_month = 2;
        s.strategies.truncate(2);
        for x in &mut s.strategies {
            x.aeza_price_multipliers.truncate(2);
        }
        s.sensitivities = vec![Sensitivity::High];
        let market: Vec<_> = crate::catalog()
            .into_iter()
            .take(1)
            .map(|p| crate::currency::plan_to_rub(p, &fx))
            .collect();
        let result = analyze(s.clone(), market.clone()).unwrap();
        for seed in 0..100 {
            let mut input = s.clone();
            input.seed = seed;
            let r = simulate(input, market.clone()).unwrap();
            for (i, run) in r.runs.iter().enumerate() {
                let p = &result.strategies[i].points[seed as usize];
                assert_eq!(p.exact_profit_rub, run.exact_totals_usd["profit"]);
                assert_eq!(p.revenue_rub, run.aeza_total_revenue_usd);
                assert_eq!(
                    p.final_active,
                    run.months.last().unwrap().providers["aeza"].active_leases
                );
            }
        }
    }
    #[test]
    fn oversized_batch_rejected_before_simulation() {
        let mut s =
            crate::currency::scenario_to_rub(crate::example(), &crate::market_data::bundled_fx());
        s.buyers_per_month = 500;
        assert!(prepare(
            Request {
                scenario: s,
                sensitivity: Sensitivity::Medium
            },
            &crate::catalog()
        )
        .unwrap_err()
        .contains("100-seed batch"));
    }
}
