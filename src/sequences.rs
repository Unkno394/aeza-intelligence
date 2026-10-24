use std::collections::{BTreeSet, HashMap};

use serde::Serialize;

use crate::historical::lcs_similarity;

const MIN_SEQUENCE_LEN: usize = 3;
const MAX_SEQUENCE_LEN: usize = 5;
const MAX_EVENTS_PER_CLIENT: usize = 96;
const MAX_GAP: usize = 1;
const CLUSTER_SIMILARITY: f64 = 0.75;

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveredPattern {
    pub sequence: Vec<String>,
    pub support: usize,
    pub support_ratio: f64,
    pub client_ids: Vec<i64>,
}

#[derive(Default)]
struct PatternCluster {
    representative: Vec<String>,
    clients: BTreeSet<i64>,
}

/// Mines short, ordered subsequences with up to one skipped event, then groups
/// near-identical candidates by LCS similarity. Support counts distinct clients.
pub fn discover_patterns(
    client_sequences: &[(i64, Vec<String>)],
    minimum_support: usize,
    limit: usize,
) -> Vec<DiscoveredPattern> {
    let eligible_clients: BTreeSet<_> = client_sequences
        .iter()
        .filter(|(_, sequence)| sequence.len() >= MIN_SEQUENCE_LEN)
        .map(|(client_id, _)| *client_id)
        .collect();
    if eligible_clients.is_empty() || limit == 0 {
        return Vec::new();
    }

    let mut support: HashMap<Vec<String>, BTreeSet<i64>> = HashMap::new();
    for (client_id, sequence) in client_sequences {
        let start = sequence.len().saturating_sub(MAX_EVENTS_PER_CLIENT);
        let events = &sequence[start..];
        let mut candidates = BTreeSet::new();
        for first in 0..events.len() {
            let mut candidate = vec![events[first].clone()];
            collect_subsequences(events, first, &mut candidate, &mut candidates);
        }
        for candidate in candidates {
            support.entry(candidate).or_default().insert(*client_id);
        }
    }

    let mut candidates: Vec<_> = support.into_iter().collect();
    candidates.sort_by(|(left, left_clients), (right, right_clients)| {
        right_clients
            .len()
            .cmp(&left_clients.len())
            .then_with(|| right.len().cmp(&left.len()))
            .then_with(|| left.cmp(right))
    });

    let mut clusters: Vec<PatternCluster> = Vec::new();
    for (sequence, clients) in candidates {
        let best = clusters
            .iter()
            .enumerate()
            .map(|(index, cluster)| (index, lcs_similarity(&sequence, &cluster.representative)))
            .filter(|(_, similarity)| *similarity >= CLUSTER_SIMILARITY)
            .max_by(|left, right| left.1.total_cmp(&right.1));
        if let Some((index, _)) = best {
            clusters[index].clients.extend(clients);
        } else {
            clusters.push(PatternCluster {
                representative: sequence,
                clients,
            });
        }
    }

    let total_clients = eligible_clients.len();
    let mut patterns: Vec<_> = clusters
        .into_iter()
        .filter(|cluster| cluster.clients.len() >= minimum_support)
        .map(|cluster| {
            let support = cluster.clients.len();
            DiscoveredPattern {
                sequence: cluster.representative,
                support,
                support_ratio: support as f64 / total_clients as f64,
                client_ids: cluster.clients.into_iter().collect(),
            }
        })
        .collect();
    patterns.sort_by(|left, right| {
        right
            .support
            .cmp(&left.support)
            .then_with(|| right.sequence.len().cmp(&left.sequence.len()))
            .then_with(|| left.sequence.cmp(&right.sequence))
    });
    patterns.truncate(limit);
    patterns
}

fn collect_subsequences(
    events: &[String],
    last_index: usize,
    candidate: &mut Vec<String>,
    output: &mut BTreeSet<Vec<String>>,
) {
    if candidate.len() >= MAX_SEQUENCE_LEN {
        return;
    }
    let last = (last_index + MAX_GAP + 1).min(events.len().saturating_sub(1));
    for next in last_index + 1..=last {
        candidate.push(events[next].clone());
        if candidate.len() >= MIN_SEQUENCE_LEN {
            output.insert(candidate.clone());
        }
        collect_subsequences(events, next, candidate, output);
        candidate.pop();
    }
}

#[cfg(test)]
mod tests {
    use super::discover_patterns;

    fn sequence(events: &[&str]) -> Vec<String> {
        events.iter().map(|event| (*event).to_owned()).collect()
    }

    #[test]
    fn discovers_repeated_scenarios_with_an_intervening_event() {
        let patterns = discover_patterns(
            &[
                (1, sequence(&["active", "prolong_wait", "active"])),
                (2, sequence(&["active", "top_up", "prolong_wait", "active"])),
                (
                    3,
                    sequence(&["active", "prolong_wait", "active", "deleted"]),
                ),
            ],
            2,
            20,
        );
        let recovery = patterns
            .iter()
            .find(|pattern| pattern.sequence == sequence(&["active", "prolong_wait", "active"]))
            .expect("recovery pattern should be mined without a hand-written template");
        assert_eq!(recovery.support, 3);
        assert_eq!(recovery.client_ids, vec![1, 2, 3]);
    }

    #[test]
    fn ignores_scenarios_seen_for_only_one_client() {
        let patterns = discover_patterns(
            &[(1, sequence(&["active", "prolong_wait", "deleted"]))],
            2,
            20,
        );
        assert!(patterns.is_empty());
    }
}
