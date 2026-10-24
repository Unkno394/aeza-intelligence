//! Boundary compatibility only. The simulation core settles exclusively in RUB.
use crate::{Plan, Report, Scenario, market_data::Fx, money};
fn amount(value: f64, from: &str, to: &str, fx: &Fx) -> f64 {
    money::float(
        money::convert_to(&value.to_string(), from, to, fx, "month", None)
            .expect("validated rates and bounded amounts"),
    )
}
pub fn plan_to_rub(mut p: Plan, fx: &Fx) -> Plan {
    p.monthly_usd = if let Some(d) = &p.details {
        if p.monthly_usd == d.normalized_monthly_usd.parse::<f64>().unwrap_or(-1.) {
            money::float(money::decimal(&d.normalized_monthly_rub).expect("verified RUB amount"))
        } else {
            amount(p.monthly_usd, "USD", "RUB", fx)
        }
    } else {
        amount(p.monthly_usd, "USD", "RUB", fx)
    };
    p
}
pub fn scenario_to_rub(mut s: Scenario, fx: &Fx) -> Scenario {
    if s.currency == "RUB" {
        return s;
    }
    s.aeza_plans = s
        .aeza_plans
        .into_iter()
        .map(|p| plan_to_rub(p, fx))
        .collect();
    for g in &mut s.segments {
        g.budget_min = amount(g.budget_min, "USD", "RUB", fx);
        g.budget_max = amount(g.budget_max, "USD", "RUB", fx);
    }
    s.aeza_variable_cost_usd = amount(s.aeza_variable_cost_usd, "USD", "RUB", fx);
    s.aeza_fixed_cost_usd = amount(s.aeza_fixed_cost_usd, "USD", "RUB", fx);
    s.currency = "RUB".into();
    s.valuation_fx = Some(fx.clone());
    s
}
pub fn legacy_report(mut r: Report, original: Scenario, fx: &Fx) -> Report {
    for run in &mut r.runs {
        for m in &mut run.months {
            for prices in [
                &mut m.aeza_effective_prices_usd,
                &mut m.effective_prices_usd,
            ] {
                for v in prices.values_mut() {
                    *v = amount(*v, "RUB", "USD", fx);
                }
            }
            for exact in m.exact_prices_usd.values_mut() {
                *exact =
                    money::text(money::convert_to(exact, "RUB", "USD", fx, "month", None).unwrap());
            }
            for ledger in m.billing_ledger_usd.values_mut() {
                for v in ledger.values_mut() {
                    let sign = if v.starts_with('-') { -1 } else { 1 };
                    *v = money::text(
                        sign * money::convert_to(
                            v.trim_start_matches('-'),
                            "RUB",
                            "USD",
                            fx,
                            "month",
                            None,
                        )
                        .unwrap(),
                    );
                }
            }
            for (name, p) in &mut m.providers {
                let ledger = &m.billing_ledger_usd[name];
                p.revenue_usd = ledger["revenue"].parse().unwrap();
                if let Some(v) = ledger.get("cost") {
                    p.cost_usd = Some(v.parse().unwrap());
                }
                if let Some(v) = ledger.get("profit") {
                    p.profit_usd = Some(v.parse().unwrap());
                }
            }
            for d in &mut m.decisions {
                d.budget_usd = amount(d.budget_usd, "RUB", "USD", fx);
                d.effective_price_usd = d.effective_price_usd.map(|v| amount(v, "RUB", "USD", fx));
            }
        }
        for v in run.exact_totals_usd.values_mut() {
            let sign = if v.starts_with('-') { -1 } else { 1 };
            *v = money::text(
                sign * money::convert_to(
                    v.trim_start_matches('-'),
                    "RUB",
                    "USD",
                    fx,
                    "month",
                    None,
                )
                .unwrap(),
            );
        }
        run.aeza_total_revenue_usd = run.exact_totals_usd["revenue"].parse().unwrap();
        run.aeza_total_profit_usd = run.exact_totals_usd["profit"].parse().unwrap();
    }
    r.scenario = original;
    r
}
/// Rename the public v2 currency fields. Source metadata always keeps original currencies.
pub fn wire(mut v: serde_json::Value, to_rub: bool) -> serde_json::Value {
    match &mut v {
        serde_json::Value::Array(a) => {
            for item in a {
                *item = wire(item.take(), to_rub)
            }
        }
        serde_json::Value::Object(o) => {
            let keys: Vec<_> = o.keys().cloned().collect();
            for key in keys {
                let item = o.remove(&key).unwrap();
                let mapped = if to_rub {
                    key.replace("_usd", "_rub")
                } else {
                    key.replace("_rub", "_usd")
                };
                o.insert(
                    mapped,
                    if key == "details" || key == "valuation_fx" || key == "fx" {
                        item
                    } else {
                        wire(item, to_rub)
                    },
                );
            }
        }
        _ => {}
    };
    v
}
