use std::collections::BTreeMap;

use aeza_intelligence::historical::{
    BehaviorEvent, HistoricalConfig, HistoricalModel, WelfordStats, lcs_similarity,
};

const HOUR: i64 = 60 * 60 * 1_000;
const DAY: i64 = 24 * HOUR;

fn servers(count: f64) -> BTreeMap<String, f64> {
    BTreeMap::from([("servers".to_owned(), count)])
}

#[test]
fn welford_tracks_sample_variance_without_history() {
    let mut stats = WelfordStats::default();
    for value in [1.0, 2.0, 3.0] {
        stats.update(value);
    }
    assert_eq!(stats.count(), 3);
    assert_eq!(stats.mean(), Some(2.0));
    assert_eq!(stats.variance(), Some(1.0));
}

#[test]
fn an_established_new_norm_stops_being_unusual() {
    let config = HistoricalConfig {
        short_half_life_ms: DAY as f64,
        long_half_life_ms: (3 * DAY) as f64,
        minimum_observations: 5,
        minimum_history_ms: 4 * DAY,
        ..HistoricalConfig::default()
    };
    let mut model = HistoricalModel::new(config);
    for day in 0..10 {
        model.observe(day * DAY, &servers(3.0), None);
    }
    let initial_change = model.observe(10 * DAY, &servers(20.0), None);
    assert!(initial_change.is_unusual);

    for day in 11..45 {
        model.observe(day * DAY, &servers(20.0), None);
    }
    let established = model.observe(45 * DAY, &servers(20.0), None);
    assert!(!established.is_unusual);
    assert!(
        model.behavior_embedding()["servers"]
            .lifetime()
            .mean()
            .unwrap()
            < 20.0
    );
}

#[test]
fn short_and_long_norms_expose_gradual_contraction() {
    let config = HistoricalConfig {
        short_half_life_ms: DAY as f64,
        long_half_life_ms: (30 * DAY) as f64,
        ..HistoricalConfig::default()
    };
    let mut model = HistoricalModel::new(config);
    for day in 0..10 {
        model.observe(day * DAY, &servers(20.0), None);
    }
    for day in 10..15 {
        model.observe(day * DAY, &servers(12.0), None);
    }
    let assessment = model.observe(15 * DAY, &servers(12.0), None);
    let trend = assessment.metric_deviations["servers"]
        .baseline_shift
        .unwrap();
    assert!(trend < 0.0);
}

#[test]
fn sparse_history_does_not_make_confident_claims() {
    let config = HistoricalConfig {
        minimum_observations: 5,
        minimum_history_ms: 4 * DAY,
        ..HistoricalConfig::default()
    };
    let mut model = HistoricalModel::new(config);
    model.observe(0, &servers(10.0), None);
    model.observe(DAY, &servers(10.0), None);
    let early_change = model.observe(2 * DAY, &servers(20.0), None);
    assert!(!early_change.is_confident);
    assert!(!early_change.is_unusual);
}

#[test]
fn an_unpaid_service_is_overdue_before_it_recovers() {
    let mut model = HistoricalModel::default();
    let empty = BTreeMap::new();
    for day in 0..5 {
        let start = day * DAY;
        for (timestamp, kind) in [
            (start, "active"),
            (start + HOUR, "prolong_wait"),
            (start + 5 * HOUR, "active"),
        ] {
            model.observe_with_event(
                timestamp,
                &empty,
                Some(BehaviorEvent {
                    stream_id: "service-a",
                    kind,
                }),
            );
        }
    }

    // A second service starts its own path and cannot add an edge from A.
    let before = model.behavior_graph().total_transitions();
    let second_service = model.observe_with_event(
        5 * DAY,
        &empty,
        Some(BehaviorEvent {
            stream_id: "service-b",
            kind: "active",
        }),
    );
    assert!(second_service.transition.is_none());
    assert_eq!(model.behavior_graph().total_transitions(), before);

    model.observe_with_event(
        5 * DAY + HOUR,
        &empty,
        Some(BehaviorEvent {
            stream_id: "service-a",
            kind: "prolong_wait",
        }),
    );
    let pending = model.assess_pending_at(7 * DAY + HOUR);
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].expected_next_event, "active");
    assert_eq!(pending[0].prior_count, 5);
    assert!(pending[0].is_overdue);
    assert_eq!(model.familiarity(), 0.0);
}

#[test]
fn the_same_wait_can_be_familiar_for_one_customer_and_new_for_another() {
    let mut habitual_late_payer = HistoricalModel::default();
    let mut habitual_on_time_payer = HistoricalModel::default();
    let empty = BTreeMap::new();
    for day in 0..5 {
        let start = day * DAY;
        for model in [&mut habitual_late_payer, &mut habitual_on_time_payer] {
            model.observe(start, &empty, Some("active"));
        }
        habitual_late_payer.observe(start + HOUR, &empty, Some("prolong_wait"));
        habitual_late_payer.observe(start + 13 * HOUR, &empty, Some("active"));
        habitual_on_time_payer.observe(start + HOUR, &empty, Some("renewed"));
        habitual_on_time_payer.observe(start + 13 * HOUR, &empty, Some("active"));
    }

    habitual_late_payer.observe(5 * DAY, &empty, Some("active"));
    habitual_on_time_payer.observe(5 * DAY, &empty, Some("active"));

    let familiar = habitual_late_payer.observe(5 * DAY + HOUR, &empty, Some("prolong_wait"));
    let new = habitual_on_time_payer.observe(5 * DAY + HOUR, &empty, Some("prolong_wait"));
    assert!(!familiar.is_unusual);
    assert!(new.is_unusual);
    assert!(new.transition.unwrap().is_new_transition);

    let pending = habitual_late_payer.assess_pending_at(5 * DAY + 13 * HOUR);
    assert_eq!(pending.len(), 1);
    assert!(!pending[0].is_overdue);
}

#[test]
fn event_only_updates_do_not_reduce_snapshot_familiarity() {
    let config = HistoricalConfig {
        minimum_observations: 2,
        minimum_history_ms: DAY,
        ..HistoricalConfig::default()
    };
    let mut model = HistoricalModel::new(config);
    model.observe(0, &servers(10.0), None);
    model.observe(DAY, &servers(10.0), None);
    assert_eq!(model.familiarity(), 1.0);
    for hour in 1..10 {
        model.observe_with_event(
            DAY + hour * HOUR,
            &BTreeMap::new(),
            Some(BehaviorEvent {
                stream_id: "service-a",
                kind: "active",
            }),
        );
    }
    assert_eq!(model.familiarity(), 1.0);
}

#[test]
fn compact_profile_survives_restart_without_replaying_history() {
    let mut model = HistoricalModel::default();
    model.observe(0, &servers(10.0), Some("active"));
    model.observe(DAY, &servers(9.0), Some("prolong_wait"));
    let saved = model.to_json().unwrap();
    let mut restored = HistoricalModel::from_json(&saved).unwrap();

    assert_eq!(
        restored
            .behavior_graph()
            .transition_count("active", "prolong_wait"),
        1
    );
    assert_eq!(
        restored.behavior_embedding()["servers"].lifetime().count(),
        2
    );
    let recovery = restored.observe(DAY + 4 * HOUR, &servers(9.0), Some("active"));
    assert_eq!(recovery.transition.unwrap().elapsed_ms, 4 * HOUR);
}

#[test]
fn lcs_matches_ordered_actions_with_an_extra_event() {
    let pattern = ["autorenew_off", "expired", "deleted"];
    let observed = ["autorenew_off", "top_up", "expired", "deleted"];
    assert_eq!(lcs_similarity(&pattern, &observed), 0.75);
    assert_eq!(lcs_similarity::<&str>(&[], &[]), 0.0);
}
