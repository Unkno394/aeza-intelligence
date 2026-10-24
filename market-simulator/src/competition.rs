//! Comparisons of archived observations. No missing specifications or prices are imputed.
use crate::{Plan, Report, Scenario};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

fn city(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .filter(|c| c.is_alphabetic())
        .collect()
}
fn eu(s: &str) -> bool {
    matches!(
        city(s).as_str(),
        "amsterdam"
            | "frankfurt"
            | "gravelines"
            | "helsinki"
            | "madrid"
            | "milan"
            | "paris"
            | "stockholm"
            | "strasbourg"
            | "warsaw"
            | "copenhagen"
    )
}
/// EU is explicitly the scenario's EU macroregion. Unknown places never match a constraint.
pub fn region_matches(regions: &[String], required: &str) -> bool {
    fn canonical(s: &str) -> String {
        match city(s).as_str() {
            "россия" | "russia" => "russia".into(),
            "москва" | "moscow" => "moscow".into(),
            "санктпетербург" | "saintpetersburg" => "saintpetersburg".into(),
            _ => city(s),
        }
    }
    regions.iter().any(|r| {
        canonical(r) == canonical(required)
            || (r.eq_ignore_ascii_case("EU") && eu(required))
            || (required.eq_ignore_ascii_case("EU") && eu(r))
            || (canonical(required) == "russia"
                && ["moscow", "saintpetersburg"].contains(&canonical(r).as_str()))
    })
}

fn default_verified() -> bool {
    true
}
fn overlaps(a: &[String], b: &[String]) -> bool {
    a.iter().any(|r| region_matches(b, r))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub scenario: Scenario,
    pub strategy: String,
    pub month: usize,
    pub region: Option<String>,
    #[serde(default = "default_verified")]
    pub verified_only: bool,
}
#[derive(Serialize)]
pub struct OfferComparison {
    pub aeza_plan_id: String,
    pub aeza_effective_price_usd: f64,
    pub competitor: Plan,
    pub effective_price_usd: Option<f64>,
    pub scenario_price_changed: bool,
    pub difference_percent: Option<f64>,
    pub comparable: bool,
    pub indicative_difference_percent: Option<f64>,
    pub resource_match: String,
    pub region_match: String,
    pub exclusion: Option<String>,
    pub rank: Option<usize>,
}
#[derive(Serialize)]
pub struct Recommendation {
    pub sensitivity: crate::Sensitivity,
    pub strategy: String,
    pub revenue_usd: f64,
    pub profit_usd: f64,
    pub final_share: f64,
    pub final_prices_usd: BTreeMap<String, f64>,
}
#[derive(Serialize)]
pub struct Analysis {
    pub report: Report,
    pub strategy: String,
    pub month: usize,
    pub region: Option<String>,
    pub comparisons: Vec<OfferComparison>,
    pub recommendations: Vec<Recommendation>,
    pub explanations: Vec<String>,
    pub segment_competitor_coverage: Vec<serde_json::Value>,
    pub limitations: Vec<String>,
}
pub fn analyze(req: Request, market: Vec<Plan>) -> Result<Analysis, String> {
    if req.month == 0
        || req.month > req.scenario.months
        || !req
            .scenario
            .strategies
            .iter()
            .any(|s| s.name == req.strategy)
    {
        return Err("select an existing strategy and month".into());
    }
    if req
        .region
        .as_ref()
        .is_some_and(|r| r.is_empty() || r.len() > 80)
    {
        return Err("invalid comparison region".into());
    }
    let report = crate::simulate(req.scenario, market.clone())?;
    let month = &report
        .runs
        .iter()
        .find(|r| r.strategy == req.strategy)
        .unwrap()
        .months[req.month - 1];
    let mut comparisons = Vec::new();
    for own in &report.scenario.aeza_plans {
        let own_price = month
            .aeza_effective_prices_usd
            .get(&own.id)
            .copied()
            .unwrap_or(own.monthly_usd);
        let mut rows = Vec::new();
        for competitor in &market {
            let exact = competitor.vcpu == own.vcpu
                && competitor.ram_gb == own.ram_gb
                && competitor.disk_gb == own.disk_gb;
            let resources = competitor.vcpu >= own.vcpu
                && competitor.vcpu <= own.vcpu * 2.
                && competitor.ram_gb >= own.ram_gb
                && competitor.ram_gb <= own.ram_gb * 2.
                && competitor.disk_gb >= own.disk_gb
                && competitor.disk_gb <= own.disk_gb * 2.;
            let location = match &req.region {
                Some(r) => {
                    region_matches(&own.regions, r) && region_matches(&competitor.regions, r)
                }
                None => overlaps(&own.regions, &competitor.regions),
            };
            let effective = month.effective_prices_usd.get(&competitor.id).copied();
            let detail = competitor.details.as_ref();
            let own_detail = own.details.as_ref();
            let verified_virtualization = own_detail
                .and_then(|d| d.virtualization.as_ref())
                .zip(detail.and_then(|d| d.virtualization.as_ref()))
                .is_some_and(|(a, b)| a == b);
            let same_service = own_detail.map_or("vps", |d| d.service_type.as_str())
                == detail.map_or("vps", |d| d.service_type.as_str());
            let same_cpu = own_detail
                .and_then(|d| d.cpu_class.as_ref())
                .zip(detail.and_then(|d| d.cpu_class.as_ref()))
                .is_some_and(|(a, b)| a == b);
            let exclusion =
                if effective.is_none() || detail.is_some_and(|d| d.availability == "unavailable") {
                    Some("scenario_unavailable")
                } else if report.scenario.market != "all"
                    && detail.is_none_or(|d| d.market != report.scenario.market)
                {
                    Some("outside_selected_market")
                } else if !report
                    .scenario
                    .accepted_service_types
                    .iter()
                    .any(|t| t == detail.map_or("vps", |d| d.service_type.as_str()))
                {
                    Some("service_type_not_requested")
                } else if !same_service && req.verified_only {
                    Some("different_service_class")
                } else if req.verified_only && !verified_virtualization {
                    Some("virtualization_unverified_or_different")
                } else if req.verified_only && !same_cpu {
                    Some("cpu_class_unverified_or_different")
                } else if req.verified_only
                    && (!detail.is_some_and(|d| d.complete_monthly_payment)
                        || !own_detail.is_some_and(|d| d.complete_monthly_payment))
                {
                    Some("total_payment_or_tax_unverified")
                } else if effective.is_none() {
                    Some("scenario_unavailable")
                } else if !resources {
                    Some("outside_resource_band")
                } else if !location {
                    Some("region_unknown_or_no_overlap")
                } else {
                    None
                };
            rows.push(OfferComparison {
                aeza_plan_id: own.id.clone(),
                aeza_effective_price_usd: own_price,
                competitor: competitor.clone(),
                effective_price_usd: effective,
                scenario_price_changed: effective.is_some_and(|v| v != competitor.monthly_usd),
                difference_percent: effective
                    .filter(|_| exclusion.is_none())
                    .map(|v| (v / own_price - 1.) * 100.),
                indicative_difference_percent: effective.map(|v| (v / own_price - 1.) * 100.),
                comparable: exclusion.is_none(),
                resource_match: if exact {
                    "exact_specs"
                } else if !resources {
                    "outside_band"
                } else {
                    "at_least_target_up_to_2x_each_resource"
                }
                .into(),
                region_match: if !location {
                    "unknown_or_mismatch"
                } else if own.regions.iter().any(|r| r == "EU")
                    || competitor.regions.iter().any(|r| r == "EU")
                {
                    "scenario_EU_macroregion"
                } else {
                    "catalog_location_overlap"
                }
                .into(),
                exclusion: exclusion.map(str::to_owned),
                rank: None,
            });
        }
        rows.sort_by(|a, b| {
            b.comparable
                .cmp(&a.comparable)
                .then_with(|| {
                    a.effective_price_usd
                        .unwrap_or(f64::INFINITY)
                        .total_cmp(&b.effective_price_usd.unwrap_or(f64::INFINITY))
                })
                .then_with(|| a.competitor.id.cmp(&b.competitor.id))
        });
        for (i, row) in rows.iter_mut().filter(|r| r.comparable).enumerate() {
            row.rank = Some(i + 1);
        }
        comparisons.extend(rows);
    }
    let mut coverage = Vec::new();
    for (i, g) in report.scenario.segments.iter().enumerate() {
        let b = crate::Buyer {
            id: 0,
            segment: i,
            budget: f64::MAX,
            plan: None,
            ever_active: false,
        };
        let cheapest = market
            .iter()
            .filter_map(|p| {
                month
                    .effective_prices_usd
                    .get(&p.id)
                    .filter(|price| crate::eligible(&report.scenario, &b, p, **price))
                    .map(|price| (p, *price))
            })
            .min_by(|(a, x), (b, y)| x.total_cmp(y).then_with(|| a.id.cmp(&b.id)));
        coverage.push(serde_json::json!({"segment":g.name,"budget_min":g.budget_min,"budget_max":g.budget_max,"currency":report.scenario.currency,"cheapest_eligible_competitor":cheapest.map(|(p,price)|serde_json::json!({"plan_id":p.id,"provider":p.provider,"price":price})),"all_buyers_below_minimum_competitor_price":cheapest.is_some_and(|(_,price)|price>g.budget_max)}));
    }
    let mut recommendations = Vec::new();
    let mut explanations = vec!["Фактический платёж и одинаковые CPU/RAM/диск не подтверждают равную производительность. В строгом рейтинге нужны известные совместимые виртуализация, класс CPU, регион и полный платёж. Непроверенные предложения видны отдельно без подтверждённого процента и ранга.".into(),"Все суммы симуляции — USD. Billing ведётся в целых nano-USD (9 знаков), конвертация по датированному ЦБ с half-even округлением один раз; точные строки доступны в billing_ledger_usd и exact_totals_usd. Цены Aéza с изменённым monthly_usd — сценарное отклонение от сохранённого оригинала.".into()];
    for &sensitivity in &report.scenario.sensitivities {
        let best = report
            .runs
            .iter()
            .filter(|r| r.sensitivity == sensitivity)
            .max_by(|a, b| a.aeza_total_profit_usd.total_cmp(&b.aeza_total_profit_usd))
            .unwrap();
        let last = best.months.last().unwrap();
        recommendations.push(Recommendation {
            sensitivity,
            strategy: best.strategy.clone(),
            revenue_usd: best.aeza_total_revenue_usd,
            profit_usd: best.aeza_total_profit_usd,
            final_share: last.providers["aeza"].market_share,
            final_prices_usd: last.aeza_effective_prices_usd.clone(),
        });
        let run = report
            .runs
            .iter()
            .find(|r| r.strategy == req.strategy && r.sensitivity == sensitivity)
            .unwrap();
        let m = &run.months[req.month - 1];
        let a = &m.providers["aeza"];
        let destinations: Vec<String> = m
            .flows
            .iter()
            .filter(|f| f.from == "aeza" && f.to != "outside")
            .map(|f| format!("{}: {}", f.to, f.events))
            .collect();
        let delta = a.active_leases as i64 - a.at_risk as i64;
        if let Some(reference) = report
            .runs
            .iter()
            .find(|r| r.strategy == "baseline" && r.sensitivity == sensitivity)
        {
            let control = &reference.months[req.month - 1].providers["aeza"];
            explanations.push(format!("{:?}, {} против baseline в месяце {}: изменение активных аренд {:+}, выручки {:+.2} USD, прибыли {:+.2} USD. Сравнение при одном seed и исходных покупателях; рыночные события и траектории продлений перечислены в сценарии.",sensitivity,req.strategy,req.month,a.active_leases as i64-control.active_leases as i64,a.revenue_usd-control.revenue_usd,a.profit_usd.unwrap()-control.profit_usd.unwrap()));
        }

        explanations.push(format!("{:?}, месяц {}: база Aéza {} → {} ({:+}). Баланс: первые аренды {} + возвращения {} + переходы внутрь {} − переходы наружу {} − без аренды {}. Получатели: {}. Из {} клиентов под риском ухода потеряно {}; вероятность экзогенного ухода не зависит от цены, выбор при продлении зависит от цены/бюджета, предпочтений и стоимости перехода.", sensitivity,req.month,a.at_risk,a.active_leases,delta,a.first_leases,a.returning_leases,a.switches_in,a.switches_out,a.churn_to_outside, if destinations.is_empty() { "переходов нет".into() } else {destinations.join(", ")},a.at_risk,a.switches_out+a.churn_to_outside));
    }
    for row in &coverage {
        if row["all_buyers_below_minimum_competitor_price"] == true {
            explanations.push(format!("Сегмент {}: максимальный бюджет {} {} ниже минимального подходящего конкурента {} {} ({}). Конкурентные переходы в этом сегменте невозможны по жёсткому ограничению бюджета; свободная мощность Aéza не делает облако доступнее.",row["segment"].as_str().unwrap(),row["budget_max"],report.scenario.currency,row["cheapest_eligible_competitor"]["price"],report.scenario.currency,row["cheapest_eligible_competitor"]["provider"].as_str().unwrap()));
        }
    }
    let explanations = explanations
        .into_iter()
        .map(|e| e.replace("USD", &report.scenario.currency))
        .collect();
    Ok(Analysis { report,strategy:req.strategy,month:req.month,region:req.region,comparisons,recommendations,segment_competitor_coverage:coverage,explanations,limitations:vec![
        "VPS Price Index / vpslocate.com, CC-BY 4.0: архивный снимок 2026-10-23, не текущие подтверждённые предложения. Исходная цена в competitor.monthly_usd; effective_price_usd — сценарная цена.".into(),
        "Сопоставимость: CPU/RAM/диск не меньше Aéza и не больше 2× по каждому ресурсу; точное совпадение помечено отдельно. Рейтинг — по общей цене подходящего тарифа, не по производительности CPU. Налоги, IP, SLA, stock и качество не проверены.".into(),
        "EU — сценарное покрытие городов ЕС: Amsterdam, Frankfurt, Gravelines, Helsinki, Madrid, Milan, Paris, Stockholm, Strasbourg, Warsaw, Copenhagen (цифры в названиях площадок игнорируются). Это допущение о доступности Aéza, не подтверждённая география. Неизвестные регионы исключены при ограничении региона.".into(),
        "Покупатель выбирает по своим минимальным ресурсам, а не по полосе сравнения тарифов: utility = 2 + предпочтение − beta × цена/бюджет − стоимость смены провайдера. beta=0.5/2/5. Бюджет и регион — жёсткие ограничения. Лучшее предложение должно превосходить outside utility. Перевыбор на каждом продлении; нет обязательных переходов.".into(),
        "Лучшая стратегия — максимум сценарной прибыли только среди переданных стратегий при тех же рыночных событиях и допущениях. Это не оценка рыночной оптимальной цены. Возврат — новая аренда ранее арендовавшего клиента; прямой переход — отдельное событие. Уникальные клиенты считаются отдельно для каждого направления, суммы направлений не являются числом уникальных людей.".into(),
        "Akash используется только для формы притока; эластичность и переходы не оценены по его данным. Cloud FinOps добавляет отдельные cloud_vm конфигурации по наблюдаемым компонентам; виртуализация этих предложений не подтверждена. VPS Hosting Radar не участвует в рейтинге: неполные CPU/RAM/диск/география, смешанные валюты, предоплата и промо. Vultr исключён из подготовленного индекса из-за отсутствующей RAM, а не из-за невыгодной цены.".into(),
        "В decisions до 8 примеров изменений выбора за месяц, а не полная выборка или причинная оценка. previous_offer_utility=null при недоступном/непосильном старом тарифе. flows содержит события и уникальных покупателей по направлению за месяц/весь прогон.".into(),
    ] })
}
