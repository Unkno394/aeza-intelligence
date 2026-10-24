use aeza_market_simulator::{
    competition::{self, Request},
    currency,
    market_data::{self, Store},
    money, *,
};
fn snapshot() -> market_data::Snapshot {
    market_data::decode(include_bytes!("../data/market-snapshot.json")).unwrap()
}
fn envelope(s: &market_data::Snapshot) -> Vec<u8> {
    let payload = serde_json::to_string(s).unwrap();
    serde_json::to_vec(&serde_json::json!({"sha256":hash(payload.as_bytes()),"payload":payload}))
        .unwrap()
}
#[test]
fn actual_cbr_cross_rates_units_and_half_even_rounding() {
    let fx = snapshot().fx;
    for (amount, from, to, expected) in [
        ("1", "USD", "RUB", "80.538100000"),
        ("1", "EUR", "RUB", "90.242900000"),
        ("80.5381", "RUB", "USD", "1.000000000"),
        ("5.93", "EUR", "USD", "6.644561977"),
        ("2973.38", "RUB", "USD", "36.918924087"),
    ] {
        assert_eq!(
            money::text(money::convert_to(amount, from, to, &fx, "month", None).unwrap()),
            expected
        );
    }
    assert_eq!(
        money::text(money::convert_to("0.04", "EUR", "RUB", &fx, "hour", Some(720)).unwrap()),
        "2598.995520000"
    );
    assert!(money::convert_to("1", "GBP", "RUB", &fx, "month", None).is_err());
    assert!(money::convert_to("1", "EUR", "RUB", &fx, "hour", None).is_err());
    assert!(money::convert_to("1", "EUR", "RUB", &fx, "year", None).is_err());
    assert_eq!(money::decimal("0.0000000005").unwrap(), 0);
    assert_eq!(money::decimal("0.0000000015").unwrap(), 2);
    assert_eq!(money::ticks(-0.0), 0);
}
#[test]
fn verified_normalization_keeps_sources_and_does_not_turn_skus_into_plans() {
    let s = snapshot();
    assert_eq!(s.plans.len(), 340);
    assert_eq!(s.raw_product_count, 633);
    assert_eq!(s.aeza_plans.len(), 110);
    assert_eq!(
        s.plans
            .iter()
            .filter(|p| p.details.as_ref().unwrap().service_type == "cloud_vm")
            .count(),
        18
    );
    let p = s
        .plans
        .iter()
        .find(|p| p.id == "finops-cloud-ru-2-4-60")
        .unwrap();
    let d = p.details.as_ref().unwrap();
    assert_eq!(d.original_price.currency, "RUB");
    assert_eq!(d.original_price.amount, "2973.38");
    assert_eq!(d.billing_hours, Some(720));
    assert_eq!(d.egress_gib, Some(100));
    assert_eq!(p.ram_gb, 4.294967296);
    assert_eq!(p.disk_gb, 64.42450944);
    assert_eq!(d.vat, "included");
    assert!(d.virtualization.is_none());
    assert_eq!(d.components.len(), 4);
    let a = s.aeza_plans.iter().find(|p| p.name == "MSKs-1").unwrap();
    assert_eq!(a.details.as_ref().unwrap().original_price.amount, "5.93");
    assert_eq!(
        a.details.as_ref().unwrap().virtualization.as_deref(),
        Some("kvm")
    );
    assert!(a.regions.contains(&"Moscow".into()));
    assert!(
        s.aeza_plans
            .iter()
            .any(|p| p.details.as_ref().unwrap().availability == "unavailable")
    );
    let mut broken = s.clone();
    broken.plans[0]
        .details
        .as_mut()
        .unwrap()
        .normalized_monthly_rub = "1".into();
    assert!(broken.validate().is_err());
    let mut broken = envelope(&s);
    broken[30] ^= 1;
    assert!(market_data::decode(&broken).is_err());
}
fn rub_scenario() -> Scenario {
    let snap = snapshot();
    let mut s = currency::scenario_to_rub(example(), &snap.fx);
    s.strategies = vec![Strategy {
        name: "baseline".into(),
        aeza_price_multipliers: vec![1.; 6],
    }];
    s.sensitivities = vec![Sensitivity::Medium];
    s.buyers_per_month = 10;
    s.monthly_departure_probability = 0.;
    s.preference_strength = 0.;
    s.aeza_plans = vec![s.aeza_plans[0].clone()];
    s.segments = vec![s.segments[0].clone()];
    s.segments[0].weight = 1.;
    s.arrival_weights = vec![1.; 6];
    s
}
#[test]
fn cpu_ram_storage_limits_are_independent_and_renewals_reserved() {
    let market = catalog()
        .into_iter()
        .map(|p| currency::plan_to_rub(p, &snapshot().fx))
        .collect::<Vec<_>>();
    for limits in [
        [1., 10000., 1e6],
        [10000., 2., 1e6],
        [10000., 10000., 30.],
        [0., 0., 0.],
    ] {
        let mut s = rub_scenario();
        s.aeza_capacity.vcpu = limits[0];
        s.aeza_capacity.ram_gb = limits[1];
        s.aeza_capacity.storage_gb = limits[2];
        s.aeza_plans[0].monthly_usd = 1.;
        s.segments[0].budget_min = 1000.;
        s.segments[0].budget_max = 1000.;
        let r = simulate(s.clone(), market.clone()).unwrap();
        for (i, m) in r.runs[0].months.iter().enumerate() {
            let u = &m.aeza_capacity_usage;
            assert!(u.vcpu <= limits[0] && u.ram_gb <= limits[1] && u.storage_gb <= limits[2]);
            assert_eq!(
                m.providers["aeza"].active_leases,
                usize::from(limits[0] > 0.)
            );
            if i > 0 && limits[0] > 0. {
                assert_eq!(m.providers["aeza"].renewals, 1);
            }
        }
    }
}
#[test]
fn monetary_ledger_accumulates_exactly_in_rubles_and_keeps_legacy_strategy() {
    let mut s = rub_scenario();
    s.months = 1;
    s.strategies[0].aeza_price_multipliers = vec![1.];
    s.arrival_weights = vec![1.];
    s.buyers_per_month = 999;
    s.aeza_plans[0].monthly_usd = 0.1;
    s.aeza_variable_cost_usd = 0.2;
    s.aeza_fixed_cost_usd = 0.3;
    s.segments[0].budget_min = 10.;
    s.segments[0].budget_max = 10.;
    let market = catalog()
        .into_iter()
        .map(|p| currency::plan_to_rub(p, &snapshot().fx))
        .collect();
    let r = simulate(s, market).unwrap();
    let ledger = &r.runs[0].months[0].billing_ledger_usd["aeza"];
    assert_eq!(ledger["revenue"], "99.900000000");
    assert_eq!(ledger["cost"], "200.100000000");
    assert_eq!(ledger["profit"], "-100.200000000");
    let legacy = simulate(example(), catalog()).unwrap();
    let baseline = legacy
        .runs
        .iter()
        .find(|r| r.strategy == "baseline" && r.sensitivity == Sensitivity::Medium)
        .unwrap();
    assert_eq!(baseline.months[5].providers["aeza"].active_leases, 146);
    assert_eq!(baseline.aeza_total_revenue_usd, 6225.);
    assert_eq!(baseline.aeza_total_profit_usd, 3966.);
}
#[test]
fn markets_regions_service_classes_virtualization_and_known_unavailability() {
    let snap = snapshot();
    let market = snap
        .plans
        .iter()
        .cloned()
        .map(|p| currency::plan_to_rub(p, &snap.fx))
        .collect::<Vec<_>>();
    let mut s = rub_scenario();
    s.market = "russia".into();
    s.accepted_service_types = vec!["vps".into(), "cloud_vm".into()];
    s.segments[0].region = Some("Russia".into());
    s.segments[0].budget_min = 10000.;
    s.segments[0].budget_max = 10000.;
    s.aeza_plans = vec![currency::plan_to_rub(
        snap.aeza_plans
            .iter()
            .find(|p| p.name == "MSKs-1")
            .unwrap()
            .clone(),
        &snap.fx,
    )];
    let r = simulate(s.clone(), market.clone()).unwrap();
    assert!(r.runs[0].months.iter().all(|m| {
        m.providers
            .iter()
            .filter(|(p, _)| ["OVHcloud", "UpCloud", "Linode"].contains(&p.as_str()))
            .all(|(_, p)| p.active_leases == 0)
    }));
    s.aeza_capacity.vcpu = 0.;
    let r = simulate(s.clone(), market.clone()).unwrap();
    assert!(
        r.runs[0].months[0]
            .providers
            .values()
            .map(|p| p.active_leases)
            .sum::<usize>()
            > 0
    );
    s.required_virtualization = Some("kvm".into());
    let r = simulate(s.clone(), market.clone()).unwrap();
    assert_eq!(r.runs[0].months[0].active_leases, 0);
    s.required_virtualization = None;
    s.aeza_capacity.vcpu = 10000.;
    s.aeza_plans = vec![currency::plan_to_rub(
        snap.aeza_plans
            .iter()
            .find(|p| p.name == "WAWs-1")
            .unwrap()
            .clone(),
        &snap.fx,
    )];
    let r = simulate(s, market).unwrap();
    assert!(
        r.runs[0]
            .months
            .iter()
            .all(|m| m.providers["aeza"].active_leases == 0)
    );
    assert!(competition::region_matches(&["Moscow".into()], "Russia"));
    assert!(!competition::region_matches(&["Russia".into()], "Moscow"));
}
#[test]
fn strict_comparison_never_assumes_unknown_hypervisor_or_taxes() {
    let snap = snapshot();
    let mut s = rub_scenario();
    s.aeza_plans = vec![currency::plan_to_rub(
        snap.aeza_plans
            .iter()
            .find(|p| p.name == "MSKs-2")
            .unwrap()
            .clone(),
        &snap.fx,
    )];
    s.accepted_service_types = vec!["vps".into(), "cloud_vm".into()];
    let market = snap
        .plans
        .iter()
        .cloned()
        .map(|p| currency::plan_to_rub(p, &snap.fx))
        .collect();
    let a = competition::analyze(
        Request {
            scenario: s,
            strategy: "baseline".into(),
            month: 1,
            region: Some("Russia".into()),
            verified_only: true,
        },
        market,
    )
    .unwrap();
    assert!(
        a.comparisons
            .iter()
            .all(|p| !p.comparable && p.rank.is_none() && p.difference_percent.is_none())
    );
}
#[test]
fn atomic_runtime_update_uses_last_verified_snapshot_without_recompile() {
    let s = snapshot();
    let path = std::env::temp_dir().join(format!("aeza-data-{}.json", std::process::id()));
    std::fs::write(&path, envelope(&s)).unwrap();
    let store = Store::new(path.clone());
    assert_eq!(store.current().0.catalog_version, s.catalog_version);
    let mut update = s.clone();
    update.catalog_version = "test-updated".into();
    std::fs::write(&path, envelope(&update)).unwrap();
    assert_eq!(store.current().0.catalog_version, "test-updated");
    std::fs::write(&path, b"corrupted").unwrap();
    let (retained, error) = store.current();
    assert_eq!(retained.catalog_version, "test-updated");
    assert!(error.is_some());
    std::fs::remove_file(path).unwrap();
}
