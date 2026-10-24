//! Aggregate every decision, not the bounded illustrative decision samples.
use crate::*;
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SegmentMonth {
    pub segment_index: usize,
    pub segment: String,
    pub decisions: usize,
    pub aeza_at_risk: usize,
    pub aeza_active: usize,
    pub loss_rate: Option<f64>,
    pub first_leases: usize,
    pub market_returns: usize,
    pub switches_in: usize,
    pub switches_out: usize,
    pub exogenous_exits: usize,
    pub choice_exits: usize,
    pub reasons: BTreeMap<String, usize>,
    pub competitor_choices: BTreeMap<String, usize>,
}
impl SegmentMonth {
    pub(super) fn record(
        &mut self,
        b: &Buyer,
        plans: &[Plan],
        next: Option<usize>,
        forced: bool,
        reason: &str,
    ) {
        self.decisions += 1;
        *self.reasons.entry(reason.into()).or_default() += 1;
        let from = b.plan.map(|i| plans[i].provider.as_str());
        let to = next.map(|i| plans[i].provider.as_str());
        self.aeza_at_risk += usize::from(from == Some("aeza"));
        self.aeza_active += usize::from(to == Some("aeza"));
        if to == Some("aeza") && from != Some("aeza") {
            if from.is_some() {
                self.switches_in += 1;
            } else if b.ever_active {
                self.market_returns += 1;
            } else {
                self.first_leases += 1;
            }
        }
        if from == Some("aeza") && to != Some("aeza") {
            if to.is_some() {
                self.switches_out += 1;
            } else if forced {
                self.exogenous_exits += 1;
            } else {
                self.choice_exits += 1;
            }
        }
        if let Some(p) = to.filter(|p| *p != "aeza") {
            *self.competitor_choices.entry(p.into()).or_default() += 1;
        }
    }
}
// A mutually exclusive funnel: first failed stage, not an alleged psychological cause.
pub(super) fn classify(
    s: &Scenario,
    b: &Buyer,
    plans: &[Plan],
    prices: &[f64],
    sensitivity: Sensitivity,
    free: [i128; 3],
    next: Option<usize>,
    forced: bool,
) -> &'static str {
    if forced {
        return "exogenous_departure_unknown_cause";
    }
    if next.is_some_and(|i| plans[i].provider == "aeza") {
        return if b.plan.is_some_and(|i| plans[i].provider == "aeza") {
            "aeza_retained"
        } else {
            "aeza_best_available_utility"
        };
    }
    let mut stage = 0;
    let mut best: Option<usize> = None;
    let g = &s.segments[b.segment];
    for (i, p) in plans
        .iter()
        .enumerate()
        .filter(|(_, p)| p.provider == "aeza")
    {
        if !policy_eligible(s, p, prices[i]) {
            continue;
        }
        stage = stage.max(1);
        if p.vcpu < g.vcpu || p.ram_gb < g.ram_gb || p.disk_gb < g.disk_gb {
            continue;
        }
        stage = stage.max(2);
        if g.region
            .as_ref()
            .is_some_and(|r| !competition::region_matches(&p.regions, r))
        {
            continue;
        }
        stage = stage.max(3);
        if prices[i] > b.budget {
            continue;
        }
        stage = stage.max(4);
        if !fits(p, free) {
            continue;
        }
        stage = stage.max(5);
        if best.is_none_or(|j| {
            utility(s, b, plans, prices, sensitivity, i)
                > utility(s, b, plans, prices, sensitivity, j)
        }) {
            best = Some(i);
        }
    }
    match stage {
        0 => "aeza_unavailable_or_policy",
        1 => "aeza_resources",
        2 => "aeza_region",
        3 => "aeza_budget",
        4 => "aeza_capacity",
        _ => {
            if let Some(j) = next {
                let i = best.unwrap();
                if utility(s, b, plans, prices, sensitivity, i)
                    == utility(s, b, plans, prices, sensitivity, j)
                {
                    return "competitor_equal_utility_id_tiebreak";
                }
                // Local diagnostic only: equalize the own offer price to the chosen competitor,
                // preserving preferences and switching penalty. Not a multi-month causal claim.
                let equal_price_score = utility(s, b, plans, prices, sensitivity, i)
                    + price_penalty(sensitivity, prices[i], b.budget)
                    - price_penalty(sensitivity, prices[j], b.budget);
                if prices[j] < prices[i]
                    && equal_price_score > utility(s, b, plans, prices, sensitivity, j)
                {
                    "competitor_price_decisive_at_equal_price"
                } else {
                    "competitor_utility_preference_or_switching"
                }
            } else {
                "aeza_utility_below_outside"
            }
        }
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub scenario: Scenario,
    pub strategy: String,
    pub sensitivity: Sensitivity,
}
/// Same cohorts, source snapshot, costs, events and seed. Only own price multipliers vary.
pub fn prepare(mut r: Request) -> Result<Scenario, String> {
    if r.scenario.currency != "RUB" {
        return Err("behavior requires RUB scenario".into());
    }
    let base = r
        .scenario
        .strategies
        .iter()
        .find(|s| s.name == r.strategy)
        .ok_or("unknown strategy")?
        .clone();
    let events = r.scenario.market_events.clone();
    r.scenario.market_events.clear();
    r.scenario.strategies.clear();
    for (name, factor) in [
        ("control", 1.),
        ("cut_20", 0.8),
        ("cut_10", 0.9),
        ("rise_15", 1.15),
    ] {
        r.scenario.strategies.push(Strategy {
            name: name.into(),
            aeza_price_multipliers: base
                .aeza_price_multipliers
                .iter()
                .map(|v| v * factor)
                .collect(),
        });
        for e in events
            .iter()
            .filter(|e| e.strategy.as_ref().is_none_or(|n| n == &base.name))
        {
            let mut e = e.clone();
            e.strategy = Some(name.into());
            r.scenario.market_events.push(e);
        }
    }
    r.scenario.sensitivities = vec![r.sensitivity];
    Ok(r.scenario)
}
pub fn analyze(s: Scenario, market: Vec<Plan>) -> Result<serde_json::Value, String> {
    let report = simulate(s, market)?;
    Ok(analyze_report(report))
}
fn profit_ticks(r: &Run) -> i128 {
    let v = &r.exact_totals_usd["profit"];
    if let Some(v) = v.strip_prefix('-') {
        -money::decimal(v).expect("engine decimal")
    } else {
        money::decimal(v).expect("engine decimal")
    }
}
pub fn analyze_report(report: Report) -> serde_json::Value {
    let control = &report.runs[0];
    let summarize = |r: &Run| {
        let a = &r.months.last().unwrap().providers["aeza"];
        let sum = |f: fn(&ProviderMetrics) -> usize| {
            r.months
                .iter()
                .map(|m| f(&m.providers["aeza"]))
                .sum::<usize>()
        };
        json!({"strategy":r.strategy,"final_clients":a.active_leases,"lease_months":sum(|a|a.active_leases),"acquisition_events":sum(|a|a.new_leases+a.switches_in),"loss_events":sum(|a|a.churn_to_outside+a.switches_out),"at_risk_months":sum(|a|a.at_risk),"revenue_rub":r.aeza_total_revenue_usd,"profit_rub":r.aeza_total_profit_usd,"exact_totals_rub":r.exact_totals_usd,"final_prices_rub":r.months.last().unwrap().exact_prices_usd.iter().filter(|(id,_)|report.scenario.aeza_plans.iter().any(|p|&p.id==*id)).collect::<BTreeMap<_,_>>()})
    };
    let mut actions = Vec::new();
    let mut reasons: BTreeMap<String, usize> = BTreeMap::new();
    for m in &control.months {
        for row in &m.buyer_behavior {
            for (k, v) in &row.reasons {
                *reasons.entry(k.clone()).or_default() += v;
            }
        }
    }
    for (key,text) in [("aeza_resources","Есть решения, где ни один доступный тариф Aéza не покрывает минимальные ресурсы. Проверьте линейку CPU/RAM/диска; снижение цены не устраняет этот этап ограничения."),("aeza_region","Есть решения без подходящего региона Aéza. Проверьте доступность нужных локаций; ценовая скидка не устраняет географическое ограничение."),("aeza_capacity","Есть решения без свободной мощности Aéza. Проверьте квоты до стимулирования спроса; инвестиционная окупаемость расширения здесь не рассчитана."),("aeza_budget","Есть решения, где подходящий тариф Aéza превышает бюджет. Проверьте результаты скидок ниже; улучшение прибыли не гарантируется."),("exogenous_departure_unknown_cause","Не назначайте случайным уходам ценовую причину: в модели это независимая вероятность. Для мер удержания нужны дополнительные наблюдения.")] {if let Some(n)=reasons.get(key){if *n>0 {actions.push(json!({"reason":key,"decision_events":n,"text":text,"status":"diagnostic_only_not_verified_intervention"}));}}}
    let candidates:Vec<_>=report.runs.iter().map(|r| {let mut v=summarize(r);
    v["segment_effects"]=json!(report.scenario.segments.iter().enumerate().map(|(index,g)|{let stats=|run:&Run|{let rows:Vec<_>=run.months.iter().flat_map(|m|&m.buyer_behavior).filter(|b|b.segment_index==index).collect();[rows.iter().map(|b|b.aeza_active).sum::<usize>() as i64,rows.iter().map(|b|b.first_leases+b.market_returns+b.switches_in).sum::<usize>() as i64,rows.iter().map(|b|b.switches_out+b.exogenous_exits+b.choice_exits).sum::<usize>() as i64]};let current=stats(r);let base=stats(control);json!({"segment":g.name,"segment_index":index,"lease_months":current[0],"delta_lease_months":current[0]-base[0],"acquisitions":current[1],"delta_acquisitions":current[1]-base[1],"losses":current[2],"delta_losses":current[2]-base[2]})}).collect::<Vec<_>>());v["delta_final_clients"]=json!(r.months.last().unwrap().providers["aeza"].active_leases as i64-control.months.last().unwrap().providers["aeza"].active_leases as i64);v["delta_profit_rub"]=json!(money::float(profit_ticks(r)-profit_ticks(control)));v}).collect();
    let best = report.runs.iter().max_by_key(|r| profit_ticks(r)).unwrap();
    let improving = profit_ticks(best) > profit_ticks(control);
    let advice = if improving {
        let label = match best.strategy.as_str() {
            "cut_20" => "снижения текущих цен на 20%",
            "cut_10" => "снижения текущих цен на 10%",
            "rise_15" => "повышения текущих цен на 15%",
            _ => "текущей стратегии",
        };
        format!("Среди проверенных цен максимальная прибыль у {label}. Используйте таблицу изменений спроса и потерь: рост прибыли не означает улучшения удержания.")
    } else {
        "Проверенные изменения цены не улучшают прибыль относительно контроля. Сохраните текущую стратегию в рамках этих допущений.".into()
    };
    json!({"currency":"RUB","recommendation":advice,"recommended_strategy":if improving{best.strategy.as_str()}else{"control"},"candidates":candidates,"diagnostic_actions":actions,"report":report,"limitations":["Причины — диагностика правил сценарной модели, а не опрос реальных клиентов. Предпочтения и стоимость перехода заданы сценарием.","Воронка Aéza показывает первый непройденный этап: доступность/политика → ресурсы → регион → бюджет → мощность → полезность. Причины исключают друг друга; бюджет не означает доказанную ценовую эластичность.","Случайный экзогенный уход не имеет установленной причины. Его вероятность не зависит от цены; число событий может меняться вслед за базой под риском.","Решения и арендо-месяцы — события; уникальные клиенты считаются отдельно по каждой причине и направлению. Уникальные группы могут пересекаться и не суммируются.","Возвращение означает повторную аренду на рынке, не обязательно возврат бывшего клиента Aéza.","Сравнение цены при одинаковой цене конкурента — локальная диагностика полезности, не обещание перехода в следующем месяце.","Рекомендации проверены на одном seed и снимке, не являются прогнозом или глобальным оптимумом. Инвестиции, ценовые реакции конкурентов и изменение затрат не моделируются."]})
}
use serde_json::json;

#[cfg(test)]
mod tests {
    use super::*;
    fn market() -> Vec<Plan> {
        let mut p = catalog().remove(0);
        p.monthly_usd = 1000000.;
        vec![p]
    }
    fn fixture() -> Scenario {
        let mut s = example();
        s.currency = "RUB".into();
        s.valuation_fx = None;
        s.market = "all".into();
        s.seed = 42;
        s.months = 3;
        s.buyers_per_month = 10;
        s.arrival_weights = vec![1.; 3];
        s.monthly_departure_probability = 0.;
        s.preference_strength = 0.;
        s.switching_cost_utility = 0.;
        s.outside_option_utility = 0.;
        s.external_models.clear();
        s.market_events.clear();
        s.strategies = vec![Strategy {
            name: "base".into(),
            aeza_price_multipliers: vec![1.; 3],
        }];
        s.sensitivities = vec![Sensitivity::Medium];
        s.aeza_plans.truncate(1);
        let p = &mut s.aeza_plans[0];
        p.monthly_usd = 100.;
        p.details = None;
        p.vcpu = 2.;
        p.ram_gb = 4.;
        p.disk_gb = 60.;
        s.segments = vec![Segment {
            name: "one".into(),
            weight: 1.,
            budget_min: 400.,
            budget_max: 400.,
            vcpu: 1.,
            ram_gb: 2.,
            disk_gb: 30.,
            region: None,
        }];
        s
    }
    #[test]
    fn full_census_balances_and_replay() {
        let s = fixture();
        let r = simulate(s.clone(), market()).unwrap();
        let run = &r.runs[0];
        assert_eq!(
            serde_json::to_value(&r).unwrap(),
            serde_json::to_value(simulate(s, market()).unwrap()).unwrap()
        );
        for m in &run.months {
            let b = &m.buyer_behavior[0];
            let a = &m.providers["aeza"];
            assert_eq!(b.decisions, m.cumulative_buyers);
            assert_eq!(b.reasons.values().sum::<usize>(), b.decisions);
            assert_eq!(b.aeza_at_risk, a.at_risk);
            assert_eq!(b.aeza_active, a.active_leases);
            assert_eq!(b.first_leases, a.first_leases);
            assert_eq!(
                b.aeza_active,
                b.aeza_at_risk + b.first_leases + b.market_returns + b.switches_in
                    - b.switches_out
                    - b.exogenous_exits
                    - b.choice_exits
            );
        }
        assert_eq!(
            run.behavior_unique_clients["aeza_best_available_utility"],
            30
        );
        assert_eq!(run.behavior_unique_clients["aeza_retained"], 20);
    }
    #[test]
    fn random_exit_is_unknown_not_price_and_returns_distinct() {
        let mut s = fixture();
        s.monthly_departure_probability = 1.;
        s.arrival_weights = vec![1., 0., 0.];
        let r = simulate(s, market()).unwrap();
        let ms = &r.runs[0].months;
        assert_eq!(ms[1].buyer_behavior[0].exogenous_exits, 10);
        assert_eq!(
            ms[1].buyer_behavior[0].reasons["exogenous_departure_unknown_cause"],
            10
        );
        assert_eq!(ms[2].buyer_behavior[0].market_returns, 10);
        assert_eq!(ms[2].buyer_behavior[0].first_leases, 0);
    }
    #[test]
    fn funnel_identifies_actual_constraints_and_utility() {
        for (expected, change) in [
            ("aeza_resources", 0),
            ("aeza_region", 1),
            ("aeza_budget", 2),
            ("aeza_capacity", 3),
            ("aeza_utility_below_outside", 4),
            ("aeza_unavailable_or_policy", 5),
        ] {
            let mut s = fixture();
            match change {
                0 => s.segments[0].vcpu = 10.,
                1 => s.segments[0].region = Some("Atlantis".into()),
                2 => s.aeza_plans[0].monthly_usd = 500.,
                3 => s.aeza_capacity.vcpu = 0.,
                4 => s.outside_option_utility = 10.,
                _ => s.accepted_service_types = vec!["cloud_vm".into()],
            };
            let r = simulate(s, market()).unwrap();
            assert_eq!(r.runs[0].months[0].buyer_behavior[0].reasons[expected], 10);
        }
    }
    #[test]
    fn competitor_price_and_preference_are_separate() {
        let s = fixture();
        let mut p = s.aeza_plans[0].clone();
        p.provider = "competitor".into();
        p.id = "zzz".into();
        p.monthly_usd = 50.;
        let b = buyer(&s, 0);
        let mut plans = vec![s.aeza_plans[0].clone(), p];
        // With identical preferences equal prices tie; the diagnostic conservatively avoids claiming decisive causality at a tie.
        let r = simulate(s.clone(), vec![plans[1].clone()]).unwrap();
        assert_eq!(
            r.runs[0].months[0].buyer_behavior[0].competitor_choices["competitor"],
            10
        );
        plans[1].monthly_usd = 100.;
        assert_eq!(
            classify(
                &s,
                &b,
                &plans,
                &[100., 100.],
                Sensitivity::Medium,
                [i128::MAX; 3],
                Some(1),
                false
            ),
            "competitor_equal_utility_id_tiebreak"
        );
    }
    #[test]
    fn recommendation_preserves_inputs_events_and_matches_direct_replay() {
        let mut s = fixture();
        let mut p = s.aeza_plans[0].clone();
        p.id = "other".into();
        p.provider = "other".into();
        p.monthly_usd = 200.;
        s.market_events.push(MarketEvent {
            plan_id: p.id.clone(),
            starts_month: 2,
            price_multiplier: 0.9,
            available: true,
            strategy: Some("base".into()),
        });
        let original = simulate(s.clone(), vec![p.clone()]).unwrap();
        let prepared = prepare(Request {
            scenario: s,
            strategy: "base".into(),
            sensitivity: Sensitivity::Medium,
        })
        .unwrap();
        assert_eq!(prepared.market_events.len(), 4);
        let r = simulate(prepared.clone(), vec![p.clone()]).unwrap();
        assert_eq!(
            serde_json::to_value(&r.runs[0].months).unwrap(),
            serde_json::to_value(&original.runs[0].months).unwrap()
        );
        let analysis = analyze(prepared, vec![p]).unwrap();
        assert_eq!(analysis["candidates"].as_array().unwrap().len(), 4);
        assert_eq!(analysis["recommended_strategy"], "rise_15");
    }
    #[test]
    fn equal_utility_choice_explains_id_tiebreak_not_price_or_preference() {
        let mut s = fixture();
        s.aeza_plans[0].id = "zzz-aeza".into();
        let mut competitor = s.aeza_plans[0].clone();
        competitor.id = "aaa-competitor".into();
        competitor.provider = "competitor".into();
        let report = simulate(s, vec![competitor]).unwrap();
        let row = &report.runs[0].months[0].buyer_behavior[0];
        assert_eq!(row.competitor_choices["competitor"], 10);
        assert_eq!(
            row.reasons.get("competitor_equal_utility_id_tiebreak"),
            Some(&10)
        );
    }
    #[test]
    fn zero_buyers_keeps_control_and_no_spurious_causes() {
        let mut s = fixture();
        s.buyers_per_month = 0;
        let a = analyze(
            prepare(Request {
                scenario: s,
                strategy: "base".into(),
                sensitivity: Sensitivity::High,
            })
            .unwrap(),
            market(),
        )
        .unwrap();
        assert_eq!(a["recommended_strategy"], "control");
        assert!(a["report"]["runs"][0]["months"][0]["buyer_behavior"]
            .as_array()
            .unwrap()
            .is_empty());
    }
}
