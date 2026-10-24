use std::collections::{BTreeMap, BTreeSet, VecDeque};

use serde::{Deserialize, Serialize};

/// A timestamp in milliseconds since the Unix epoch.
pub type Timestamp = i64;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct HistoricalConfig {
    /// Feature names expected in a complete snapshot. When empty, any
    /// non-empty feature map is treated as complete.
    pub expected_features: BTreeSet<String>,
    pub short_half_life_ms: f64,
    pub long_half_life_ms: f64,
    pub minimum_observations: u64,
    pub minimum_history_ms: i64,
    pub minimum_familiarity: f64,
    pub anomaly_z_threshold: f64,
    pub sequence_capacity: usize,
    /// Minimum outgoing observations before a transition can be called rare.
    pub minimum_transition_observations: u64,
    /// States whose unusually long duration should be checked before exit.
    pub watched_states: BTreeSet<String>,
    pub minimum_overdue_ms: i64,
    pub overdue_ratio: f64,
}

impl Default for HistoricalConfig {
    fn default() -> Self {
        Self {
            expected_features: BTreeSet::new(),
            short_half_life_ms: 7.0 * 24.0 * 60.0 * 60.0 * 1_000.0,
            long_half_life_ms: 90.0 * 24.0 * 60.0 * 60.0 * 1_000.0,
            minimum_observations: 10,
            minimum_history_ms: 30 * 24 * 60 * 60 * 1_000,
            minimum_familiarity: 0.5,
            anomaly_z_threshold: 3.0,
            sequence_capacity: 256,
            minimum_transition_observations: 5,
            watched_states: BTreeSet::from(["prolong_wait".to_owned()]),
            minimum_overdue_ms: 60 * 60 * 1_000,
            overdue_ratio: 1.5,
        }
    }
}

/// Welford's online mean and sample variance. Memory use is constant regardless
/// of the number of observations.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct WelfordStats {
    count: u64,
    mean: f64,
    m2: f64,
}

impl WelfordStats {
    pub fn count(&self) -> u64 {
        self.count
    }

    pub fn mean(&self) -> Option<f64> {
        (self.count > 0).then_some(self.mean)
    }

    pub fn variance(&self) -> Option<f64> {
        (self.count > 1).then_some(self.m2.max(0.0) / (self.count - 1) as f64)
    }

    pub fn standard_deviation(&self) -> Option<f64> {
        self.variance().map(f64::sqrt)
    }

    pub fn update(&mut self, value: f64) {
        if !value.is_finite() {
            return;
        }
        self.count += 1;
        let delta = value - self.mean;
        self.mean += delta / self.count as f64;
        let delta_after_update = value - self.mean;
        self.m2 += delta * delta_after_update;
    }

    /// Returns an absolute z-score. A constant historical value still detects
    /// a different new value, using a finite capped score.
    pub fn absolute_z_score(&self, value: f64) -> Option<f64> {
        if !value.is_finite() || self.count < 2 {
            return None;
        }
        let deviation = (value - self.mean).abs();
        let stddev = self.standard_deviation().unwrap_or(0.0);
        Some(if stddev <= f64::EPSILON {
            if deviation <= f64::EPSILON { 0.0 } else { 12.0 }
        } else {
            (deviation / stddev).min(12.0)
        })
    }
}

/// Exponentially weighted mean and variance used for concept drift.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
struct DecayedStats {
    count: u64,
    mean: f64,
    variance: f64,
    last_timestamp: Option<Timestamp>,
}

impl DecayedStats {
    fn update(&mut self, value: f64, timestamp: Timestamp, half_life_ms: f64) {
        if !value.is_finite() {
            return;
        }
        self.count += 1;
        if self.count == 1 {
            self.mean = value;
            self.last_timestamp = Some(timestamp);
            return;
        }

        let alpha = match self.last_timestamp {
            Some(last) if timestamp > last && half_life_ms.is_finite() && half_life_ms > 0.0 => {
                1.0 - (-std::f64::consts::LN_2 * timestamp.saturating_sub(last) as f64
                    / half_life_ms)
                    .exp()
            }
            _ => 1.0 / self.count as f64,
        }
        .clamp(0.001, 1.0);
        let delta = value - self.mean;
        self.mean += alpha * delta;
        self.variance = (1.0 - alpha) * (self.variance + alpha * delta * delta);
        self.last_timestamp = Some(timestamp);
    }

    fn absolute_z_score(&self, value: f64) -> Option<f64> {
        if !value.is_finite() || self.count < 2 {
            return None;
        }
        let deviation = (value - self.mean).abs();
        let stddev = self.variance.max(0.0).sqrt();
        Some(if stddev <= f64::EPSILON {
            if deviation <= f64::EPSILON { 0.0 } else { 12.0 }
        } else {
            (deviation / stddev).min(12.0)
        })
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct BehaviorEmbedding {
    lifetime: WelfordStats,
    short_term: DecayedStats,
    long_term: DecayedStats,
}

impl BehaviorEmbedding {
    pub fn lifetime(&self) -> &WelfordStats {
        &self.lifetime
    }

    pub fn short_term_mean(&self) -> Option<f64> {
        (self.short_term.count > 0).then_some(self.short_term.mean)
    }

    pub fn long_term_mean(&self) -> Option<f64> {
        (self.long_term.count > 0).then_some(self.long_term.mean)
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
struct TransitionProfile {
    count: u64,
    elapsed_ms: WelfordStats,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct BehaviorGraph {
    transitions: BTreeMap<String, BTreeMap<String, TransitionProfile>>,
    outgoing_counts: BTreeMap<String, u64>,
    total_transitions: u64,
}

impl BehaviorGraph {
    pub fn transition_count(&self, from: &str, to: &str) -> u64 {
        self.transitions
            .get(from)
            .and_then(|edges| edges.get(to))
            .map_or(0, |profile| profile.count)
    }

    pub fn total_transitions(&self) -> u64 {
        self.total_transitions
    }

    pub fn transition_probability(&self, from: &str, to: &str) -> Option<f64> {
        self.outgoing_counts
            .get(from)
            .map(|outgoing| self.transition_count(from, to) as f64 / *outgoing as f64)
    }
}

/// An event belongs to one service or other independent event stream.
#[derive(Clone, Copy, Debug)]
pub struct BehaviorEvent<'a> {
    pub stream_id: &'a str,
    pub kind: &'a str,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct MetricDeviation {
    pub lifetime_z_score: Option<f64>,
    pub short_term_z_score: Option<f64>,
    pub long_term_z_score: Option<f64>,
    pub short_term_mean: Option<f64>,
    pub long_term_mean: Option<f64>,
    /// Signed difference between recent and long-term norms, in feature units.
    /// For `server_count`, a negative value means gradual contraction.
    pub baseline_shift: Option<f64>,
    /// Largest available absolute deviation, capped at 12 standard deviations.
    pub score: Option<f64>,
    pub observations: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct TransitionDeviation {
    pub stream_id: String,
    pub from: String,
    pub to: String,
    pub elapsed_ms: i64,
    pub prior_count: u64,
    pub outgoing_count: u64,
    pub transition_probability: Option<f64>,
    pub rarity_score: Option<f64>,
    pub confidence: f64,
    pub expected_elapsed_ms: Option<f64>,
    pub elapsed_z_score: Option<f64>,
    /// True when this edge has not appeared before in a sufficiently populated graph.
    pub is_new_transition: bool,
    pub is_rare_transition: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PendingStateDeviation {
    pub stream_id: String,
    pub state: String,
    pub waiting_ms: i64,
    pub expected_next_event: String,
    pub prior_count: u64,
    pub typical_wait_ms: f64,
    pub overdue_after_ms: f64,
    pub is_overdue: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct HistoricalAssessment {
    /// Confidence in the accumulated customer profile, from 0 to 1.
    pub familiarity: f64,
    /// Confidence in the relevant event history, from 0 to 1.
    pub event_familiarity: f64,
    pub is_confident: bool,
    pub metric_deviations: BTreeMap<String, MetricDeviation>,
    pub transition: Option<TransitionDeviation>,
    pub pending_states: Vec<PendingStateDeviation>,
    /// Largest metric or transition z-score. This is not a churn probability;
    /// a new graph edge can also make `is_unusual` true without a z-score.
    pub anomaly_score: Option<f64>,
    pub is_unusual: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct HistoricalModel {
    config: HistoricalConfig,
    features: BTreeMap<String, BehaviorEmbedding>,
    graph: BehaviorGraph,
    last_events: BTreeMap<String, (String, Timestamp)>,
    recent_events: VecDeque<(String, String)>,
    observation_count: u64,
    first_timestamp: Option<Timestamp>,
    last_timestamp: Option<Timestamp>,
    complete_observations: u64,
}

impl Default for HistoricalModel {
    fn default() -> Self {
        Self::new(HistoricalConfig::default())
    }
}

impl HistoricalModel {
    pub fn new(config: HistoricalConfig) -> Self {
        Self {
            config,
            features: BTreeMap::new(),
            graph: BehaviorGraph::default(),
            last_events: BTreeMap::new(),
            recent_events: VecDeque::new(),
            observation_count: 0,
            first_timestamp: None,
            last_timestamp: None,
            complete_observations: 0,
        }
    }

    /// Serialize only the compact profile, graph and recent event window.
    /// The full observation history is not needed to resume online updates.
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }

    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }

    pub fn behavior_graph(&self) -> &BehaviorGraph {
        &self.graph
    }

    pub fn behavior_embedding(&self) -> &BTreeMap<String, BehaviorEmbedding> {
        &self.features
    }

    pub fn recent_events(&self) -> impl Iterator<Item = &str> {
        self.recent_events.iter().map(|(_, kind)| kind.as_str())
    }

    pub fn recent_events_for<'a>(&'a self, stream_id: &'a str) -> impl Iterator<Item = &'a str> {
        self.recent_events
            .iter()
            .filter(move |(stream, _)| stream == stream_id)
            .map(|(_, kind)| kind.as_str())
    }

    /// Check ongoing watched states without recording a new snapshot or event.
    pub fn assess_pending_at(&self, timestamp: Timestamp) -> Vec<PendingStateDeviation> {
        self.pending_states_at(timestamp, None)
    }

    /// Score a snapshot against prior history, then incorporate it into the
    /// customer's profile. `event` is optional because periodic snapshots do
    /// not always reveal the exact intermediate state transitions.
    pub fn observe(
        &mut self,
        timestamp: Timestamp,
        metrics: &BTreeMap<String, f64>,
        event: Option<&str>,
    ) -> HistoricalAssessment {
        self.observe_with_event(
            timestamp,
            metrics,
            event.map(|kind| BehaviorEvent {
                stream_id: "customer",
                kind,
            }),
        )
    }

    /// Use a separate stream ID for each service. Only observed events update
    /// the graph; a snapshot without an event leaves transition history alone.
    pub fn observe_with_event(
        &mut self,
        timestamp: Timestamp,
        metrics: &BTreeMap<String, f64>,
        event: Option<BehaviorEvent<'_>>,
    ) -> HistoricalAssessment {
        let familiarity = self.familiarity();
        let mut metric_deviations = BTreeMap::new();
        let mut anomaly_score: Option<f64> = None;

        for (name, value) in metrics {
            if !value.is_finite() {
                continue;
            }
            if let Some(profile) = self.features.get(name) {
                let lifetime_z_score = profile.lifetime.absolute_z_score(*value);
                let short_term_z_score = profile.short_term.absolute_z_score(*value);
                let long_term_z_score = profile.long_term.absolute_z_score(*value);
                let short_term_mean = profile.short_term_mean();
                let long_term_mean = profile.long_term_mean();
                let baseline_shift = short_term_mean
                    .zip(long_term_mean)
                    .map(|(short, long)| short - long);
                // Lifetime statistics remain available for inspection, but
                // decayed profiles decide whether a new norm is still unusual.
                let score = [short_term_z_score, long_term_z_score]
                    .into_iter()
                    .flatten()
                    .reduce(f64::max);
                if let Some(score) = score {
                    anomaly_score = Some(anomaly_score.map_or(score, |old| old.max(score)));
                }
                metric_deviations.insert(
                    name.clone(),
                    MetricDeviation {
                        lifetime_z_score,
                        short_term_z_score,
                        long_term_z_score,
                        short_term_mean,
                        long_term_mean,
                        baseline_shift,
                        score,
                        observations: profile.lifetime.count(),
                    },
                );
            }
        }

        let transition = event.and_then(|incoming| self.transition_deviation(timestamp, incoming));
        if let Some(score) = transition.as_ref().and_then(|value| value.elapsed_z_score) {
            anomaly_score = Some(anomaly_score.map_or(score, |old| old.max(score)));
        }
        let pending_states = self.pending_states_at(timestamp, event.map(|value| value.stream_id));
        let event_familiarity = transition
            .as_ref()
            .map_or(0.0, |value| value.confidence)
            .max(
                pending_states
                    .iter()
                    .map(|value| {
                        (value.prior_count as f64
                            / self.config.minimum_transition_observations.max(1) as f64)
                            .min(1.0)
                    })
                    .fold(0.0, f64::max),
            );
        let metric_unusual = familiarity >= self.config.minimum_familiarity
            && metric_deviations.values().any(|value| {
                value
                    .score
                    .is_some_and(|score| score >= self.config.anomaly_z_threshold)
            });
        let event_unusual = transition.as_ref().is_some_and(|value| {
            value.confidence >= self.config.minimum_familiarity
                && (value.is_new_transition
                    || value.is_rare_transition
                    || value
                        .elapsed_z_score
                        .is_some_and(|score| score >= self.config.anomaly_z_threshold))
        }) || pending_states.iter().any(|value| value.is_overdue);
        let is_confident = familiarity >= self.config.minimum_familiarity
            || event_familiarity >= self.config.minimum_familiarity;
        let is_unusual = metric_unusual || event_unusual;

        // Update after scoring so the current sample cannot make itself look normal.
        if metrics.values().any(|value| value.is_finite()) {
            self.observation_count += 1;
            self.first_timestamp = Some(
                self.first_timestamp
                    .map_or(timestamp, |old| old.min(timestamp)),
            );
            self.last_timestamp = Some(
                self.last_timestamp
                    .map_or(timestamp, |old| old.max(timestamp)),
            );
            if self.snapshot_is_complete(metrics) {
                self.complete_observations += 1;
            }
        }
        for (name, value) in metrics {
            if !value.is_finite() {
                continue;
            }
            let profile = self.features.entry(name.clone()).or_default();
            profile.lifetime.update(*value);
            profile
                .short_term
                .update(*value, timestamp, self.config.short_half_life_ms);
            profile
                .long_term
                .update(*value, timestamp, self.config.long_half_life_ms);
        }
        if let Some(incoming) = event {
            self.record_event(timestamp, incoming);
        }

        HistoricalAssessment {
            familiarity,
            event_familiarity,
            is_confident,
            metric_deviations,
            transition,
            pending_states,
            anomaly_score,
            is_unusual,
        }
    }

    fn transition_deviation(
        &self,
        timestamp: Timestamp,
        incoming: BehaviorEvent<'_>,
    ) -> Option<TransitionDeviation> {
        let (previous, previous_at) = self.last_events.get(incoming.stream_id)?;
        if timestamp < *previous_at {
            return None;
        }
        let elapsed_ms = timestamp.saturating_sub(*previous_at);
        let profile = self
            .graph
            .transitions
            .get(previous)
            .and_then(|edges| edges.get(incoming.kind));
        let prior_count = profile.map_or(0, |value| value.count);
        let outgoing_count = self
            .graph
            .outgoing_counts
            .get(previous)
            .copied()
            .unwrap_or(0);
        let distinct_destinations = self
            .graph
            .transitions
            .get(previous)
            .map_or(0, BTreeMap::len) as f64;
        let transition_probability = self.graph.transition_probability(previous, incoming.kind);
        let rarity_score = (outgoing_count > 0).then(|| {
            let smoothed =
                (prior_count as f64 + 1.0) / (outgoing_count as f64 + distinct_destinations + 1.0);
            (1.0 - smoothed) * (outgoing_count as f64 / 20.0).min(1.0)
        });
        let confidence = (outgoing_count as f64
            / self.config.minimum_transition_observations.max(1) as f64)
            .min(1.0);
        Some(TransitionDeviation {
            stream_id: incoming.stream_id.to_owned(),
            from: previous.clone(),
            to: incoming.kind.to_owned(),
            elapsed_ms,
            prior_count,
            outgoing_count,
            transition_probability,
            rarity_score,
            confidence,
            expected_elapsed_ms: profile.and_then(|value| value.elapsed_ms.mean()),
            elapsed_z_score: profile
                .and_then(|value| value.elapsed_ms.absolute_z_score(elapsed_ms as f64)),
            is_new_transition: prior_count == 0
                && outgoing_count >= self.config.minimum_transition_observations,
            is_rare_transition: prior_count > 0
                && outgoing_count >= self.config.minimum_transition_observations
                && transition_probability.is_some_and(|probability| probability < 0.1),
        })
    }

    fn pending_states_at(
        &self,
        timestamp: Timestamp,
        excluded_stream: Option<&str>,
    ) -> Vec<PendingStateDeviation> {
        self.last_events
            .iter()
            .filter(|(stream, (state, _))| {
                Some(stream.as_str()) != excluded_stream
                    && self.config.watched_states.contains(state)
            })
            .filter_map(|(stream, (state, entered_at))| {
                let (expected_next, edge) = self
                    .graph
                    .transitions
                    .get(state)?
                    .iter()
                    .filter(|(_, profile)| {
                        profile.count >= self.config.minimum_transition_observations
                    })
                    .max_by_key(|(_, profile)| profile.count)?;
                let typical_wait_ms = edge.elapsed_ms.mean()?;
                let stddev = edge.elapsed_ms.standard_deviation().unwrap_or(0.0);
                let margin = (self.config.anomaly_z_threshold.max(0.0) * stddev)
                    .max(typical_wait_ms * (self.config.overdue_ratio.max(1.0) - 1.0))
                    .max(self.config.minimum_overdue_ms.max(0) as f64);
                let overdue_after_ms = typical_wait_ms + margin;
                let waiting_ms = timestamp.saturating_sub(*entered_at).max(0);
                Some(PendingStateDeviation {
                    stream_id: stream.clone(),
                    state: state.clone(),
                    waiting_ms,
                    expected_next_event: expected_next.clone(),
                    prior_count: edge.count,
                    typical_wait_ms,
                    overdue_after_ms,
                    is_overdue: waiting_ms as f64 > overdue_after_ms,
                })
            })
            .collect()
    }

    fn record_event(&mut self, timestamp: Timestamp, incoming: BehaviorEvent<'_>) {
        if let Some((previous, previous_at)) = self.last_events.get(incoming.stream_id) {
            if timestamp < *previous_at {
                return;
            }
            let elapsed_ms = timestamp.saturating_sub(*previous_at) as f64;
            let profile = self
                .graph
                .transitions
                .entry(previous.clone())
                .or_default()
                .entry(incoming.kind.to_owned())
                .or_default();
            profile.count += 1;
            profile.elapsed_ms.update(elapsed_ms);
            *self
                .graph
                .outgoing_counts
                .entry(previous.clone())
                .or_default() += 1;
            self.graph.total_transitions += 1;
        }
        self.last_events.insert(
            incoming.stream_id.to_owned(),
            (incoming.kind.to_owned(), timestamp),
        );
        if self.config.sequence_capacity > 0 {
            self.recent_events
                .push_back((incoming.stream_id.to_owned(), incoming.kind.to_owned()));
            while self.recent_events.len() > self.config.sequence_capacity {
                self.recent_events.pop_front();
            }
        }
    }

    pub fn familiarity(&self) -> f64 {
        if self.observation_count == 0 {
            return 0.0;
        }
        let observation_factor = if self.config.minimum_observations == 0 {
            1.0
        } else {
            (self.observation_count as f64 / self.config.minimum_observations as f64).min(1.0)
        };
        let duration_ms = match (self.first_timestamp, self.last_timestamp) {
            (Some(first), Some(last)) => last.saturating_sub(first).max(0),
            _ => 0,
        };
        let duration_factor = if self.config.minimum_history_ms <= 0 {
            1.0
        } else {
            (duration_ms as f64 / self.config.minimum_history_ms as f64).min(1.0)
        };
        let completeness_factor = self.complete_observations as f64 / self.observation_count as f64;
        (observation_factor * duration_factor * completeness_factor).clamp(0.0, 1.0)
    }

    fn snapshot_is_complete(&self, metrics: &BTreeMap<String, f64>) -> bool {
        if self.config.expected_features.is_empty() {
            return !metrics.is_empty();
        }
        self.config
            .expected_features
            .iter()
            .all(|name| metrics.get(name).is_some_and(|value| value.is_finite()))
    }
}

/// Longest common subsequence length, using O(min(a.len(), b.len())) memory.
pub fn lcs_length<T: Eq>(a: &[T], b: &[T]) -> usize {
    let (shorter, longer) = if a.len() <= b.len() { (a, b) } else { (b, a) };
    let mut row = vec![0usize; shorter.len() + 1];
    for item in longer {
        let mut diagonal = 0;
        for (index, candidate) in shorter.iter().enumerate() {
            let above = row[index + 1];
            row[index + 1] = if item == candidate {
                diagonal + 1
            } else {
                row[index + 1].max(row[index])
            };
            diagonal = above;
        }
    }
    row[shorter.len()]
}

/// LCS similarity normalized by the longer sequence length, from 0 to 1.
/// Empty sequences carry no evidence of a matching behavioral pattern.
pub fn lcs_similarity<T: Eq>(a: &[T], b: &[T]) -> f64 {
    let longest = a.len().max(b.len());
    if longest == 0 {
        0.0
    } else {
        lcs_length(a, b) as f64 / longest as f64
    }
}
