# Aéza Historical Model

This crate contains an online behavioral model for one customer at a time. It
adapts the Welford embedding, transition graph, and LCS ideas from the local
`/home/user/Documents/antifrod/backend/src/fraud` project to service and
payment behavior. No training data is required before the first observation.

## Inputs

- Pass a numeric feature map to `HistoricalModel::observe` for each customer
  snapshot. Useful feature names include `server_count`, `monthly_spend`,
  `financial_runway_days`, and `autorenew_disabled_share`. Keep each feature's
  units consistent over time.
- Pass real lifecycle or payment events with `observe_with_event`. Use the
  service ID as `stream_id` so transitions from separate services are not
  joined. An event-only update can use an empty feature map.
- Periodic snapshots without an event must pass `None`. The model cannot infer
  intermediate transitions that were not observed.
- Pass observations for a customer in timestamp order. Timestamps are Unix
  milliseconds.

```rust
use std::collections::BTreeMap;
use aeza_intelligence::historical::{BehaviorEvent, HistoricalModel};

fn main() -> Result<(), Box<dyn std::error::Error>> {
let mut model = HistoricalModel::default();
let features = BTreeMap::from([
    ("server_count".to_owned(), 15.0),
    ("financial_runway_days".to_owned(), 20.0),
]);

let assessment = model.observe_with_event(
    1_780_000_000_000,
    &features,
    Some(BehaviorEvent {
        stream_id: "service-42",
        kind: "prolong_wait",
    }),
);
let _ = assessment.familiarity;

// Check a waiting service even when no new event has arrived.
let pending = model.assess_pending_at(1_780_000_000_000 + 48 * 60 * 60 * 1_000);
let _ = pending;

// Store this compact JSON with the customer's record; restore it later.
let saved = model.to_json()?;
let restored = HistoricalModel::from_json(&saved)?;
let _ = restored;
Ok(())
}
```

## Interpretation

`familiarity` uses snapshot count, history duration, and completeness.
`event_familiarity` uses relevant transition history. Low familiarity prevents
the model from making a confident claim. `is_unusual` means a customer's
current behavior differs from their own history; it is not a churn probability.
The Snapshot Model remains the source for current-state risk, especially when
the historical profile is still sparse.

The embedding keeps lifetime Welford statistics plus short and long profiles
with exponential time decay. Only the decayed profiles decide current
deviation, so an established new habit can become normal. Each metric also
reports `baseline_shift` (recent mean minus long-term mean), which shows the
direction of gradual changes such as declining server count. The graph records
transition frequency and elapsed time. Watched states such as `prolong_wait`
are checked for unusually long waits before a recovery event occurs.

`lcs_similarity` compares event sequences with a supplied pattern. It only
reports sequence similarity; pattern labels and their relationship to churn
must come from separate evidence.

The library provides JSON export and import, while the application chooses
where to store that JSON. The current repository has no customer data source,
Snapshot Model, or storage adapter.
