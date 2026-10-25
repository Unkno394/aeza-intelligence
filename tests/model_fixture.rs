//! Both models over 60 generated days of 10 clients
//! (scripts/fixtures/model_history.json, made by scripts/given/generate.py).
//! The generator changes the behavior of the first account partway through.

use std::collections::{BTreeMap, BTreeSet};

use aeza_intelligence::{
    analysis::{self, Params},
    historical::HistoricalModel,
    model::ClientFeatures,
};
use chrono::{DateTime, Utc};
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Day {
    observed_at: DateTime<Utc>,
    clients: Vec<ClientFeatures>,
}

fn history() -> Vec<Day> {
    let raw = include_str!("../scripts/fixtures/model_history.json");
    serde_json::from_str(raw).unwrap()
}

fn feature_map(client: &ClientFeatures) -> BTreeMap<String, f64> {
    let mut values = BTreeMap::from([
        ("services_count".into(), client.services_count as f64),
        ("monthly_spend".into(), client.monthly_spend),
        ("spendable_balance".into(), client.spendable_balance as f64),
        ("upcoming_payments".into(), client.upcoming_payments as f64),
    ]);
    let optional = [
        ("payment_coverage", client.payment_coverage),
        ("no_auto_prolong_share", client.no_auto_prolong_share),
        ("prolong_wait_share", client.prolong_wait_share),
        ("failed_prolong_share", client.failed_prolong_share),
        ("expiring_revenue_share", client.expiring_revenue_share),
    ];
    for (name, value) in optional {
        if let Some(value) = value.filter(|value| value.is_finite()) {
            values.insert(name.into(), value);
        }
    }
    values
}

#[test]
fn fixture_is_a_dated_history_of_the_same_clients() {
    let days = history();
    assert!(days.len() >= 31, "{} days", days.len());
    let accounts: BTreeSet<i64> = days[0].clients.iter().map(|c| c.account_id).collect();
    assert_eq!(accounts.len(), days[0].clients.len(), "account IDs repeat");
    for pair in days.windows(2) {
        assert!(pair[1].observed_at > pair[0].observed_at, "dates must increase");
        let next: BTreeSet<i64> = pair[1].clients.iter().map(|c| c.account_id).collect();
        assert_eq!(next, accounts);
    }
}

#[test]
fn historical_model_becomes_confident_and_flags_the_changed_account() {
    let days = history();
    let changed_account = days[0].clients[0].account_id;
    let mut models: BTreeMap<i64, HistoricalModel> = BTreeMap::new();
    let mut last = BTreeMap::new();
    for (index, day) in days.iter().enumerate() {
        for client in &day.clients {
            let model = models.entry(client.account_id).or_default();
            let assessment =
                model.observe(day.observed_at.timestamp_millis(), &feature_map(client), None);
            last.insert(client.account_id, assessment);
        }
        // Halfway through, every model goes through its persisted JSON form, as
        // between two client_snapshot runs.
        if index == days.len() / 2 {
            for model in models.values_mut() {
                *model = HistoricalModel::from_json(&model.to_json().unwrap()).unwrap();
            }
        }
    }
    let unconfident: Vec<_> = last
        .iter()
        .filter_map(|(id, assessment)| (!assessment.is_confident).then_some(id))
        .collect();
    assert!(unconfident.is_empty(), "not confident after {} days: {unconfident:?}", days.len());
    assert!(
        last[&changed_account].is_unusual,
        "the generated change of account {changed_account} was missed"
    );
}

#[test]
fn snapshot_analysis_scores_every_client() {
    let days = history();
    let clients = &days.last().unwrap().clients;
    let report = analysis::analyze(clients, Params { k: 3, shrinkage: 0.3 });
    assert_eq!(report.clients.len(), clients.len());
    assert_eq!(report.features.len(), 5);
    for client in &report.clients {
        assert!(client.robust_z.iter().all(|value| value.is_finite()));
    }
}
