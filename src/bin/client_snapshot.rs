use std::{collections::BTreeMap, time::Duration};

use aeza_intelligence::{
    PgAdapter,
    historical::{BehaviorEvent, HISTORICAL_MODEL_VERSION, HistoricalAssessment, HistoricalModel},
    model::ClientFeatures,
};
use anyhow::{Context, Result, bail, ensure};
use chrono::{DateTime, Utc};
use clap::Parser;
use serde_json::Value;
use sqlx::Row;

#[derive(Parser)]
#[command(about = "Save one daily client feature and historical-model snapshot")]
struct Cli {
    #[arg(long, env = "DATABASE_URL", hide_env_values = true)]
    database_url: String,
    /// Keep checking hourly; each account gets at most one snapshot per UTC day.
    #[arg(long)]
    schedule: bool,
    /// Test one fixture account at a controlled UTC instant.
    #[arg(long)]
    as_of: Option<DateTime<Utc>>,
    #[arg(long, allow_hyphen_values = true)]
    account_id: Option<i64>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    if cli.schedule && (cli.as_of.is_some() || cli.account_id.is_some()) {
        bail!("--schedule cannot be combined with a controlled date or account");
    }
    if cli.as_of.is_some() != cli.account_id.is_some() {
        bail!("--as-of and --account-id must be supplied together");
    }
    let adapter = PgAdapter::connect(&cli.database_url).await?;
    adapter.migrate().await?;
    if let Some(account_id) = cli.account_id {
        ensure!(
            account_id < 0,
            "controlled dates are restricted to negative fixture IDs"
        );
        let email: Option<String> =
            sqlx::query_scalar("SELECT email FROM public.accounts WHERE id = $1")
                .bind(account_id)
                .fetch_optional(adapter.pool())
                .await?;
        ensure!(
            email.as_deref().is_some_and(
                |email| email.starts_with("model-fixture-") && email.ends_with("@example.test")
            ),
            "controlled dates require a generated model fixture account"
        );
    }
    loop {
        let failed =
            match collect_at(&adapter, cli.as_of.unwrap_or_else(Utc::now), cli.account_id).await {
                Ok(count) => {
                    println!("saved {count} client snapshots");
                    false
                }
                Err(error) if cli.schedule => {
                    eprintln!("client snapshot failed: {error:#}; retrying in 60 seconds");
                    true
                }
                Err(error) => return Err(error),
            };
        if !cli.schedule {
            break;
        }
        tokio::time::sleep(Duration::from_secs(if failed { 60 } else { 3600 })).await;
    }
    Ok(())
}

async fn collect_at(
    adapter: &PgAdapter,
    now: DateTime<Utc>,
    account_id: Option<i64>,
) -> Result<usize> {
    let day = now.date_naive();
    let clients = adapter.client_features(now, "7 days").await?;
    let event_highwater: i64 = sqlx::query_scalar(
        "SELECT COALESCE(MAX(id), 0) FROM public.operation_logs WHERE \"occurredAt\" <= $1",
    )
    .bind(now)
    .fetch_one(adapter.pool())
    .await?;
    let mut count = 0;
    for client in clients
        .into_iter()
        .filter(|client| account_id.is_none_or(|id| client.account_id == id))
    {
        let same_day_state: Option<String> = sqlx::query_scalar(
            "SELECT model_state::text FROM public.client_snapshots \
             WHERE account_id = $1 AND observed_on = $2",
        )
        .bind(client.account_id)
        .bind(day)
        .fetch_optional(adapter.pool())
        .await?;
        let same_day_version = same_day_state
            .as_deref()
            .and_then(|state| serde_json::from_str::<Value>(state).ok())
            .and_then(|state| state.get("model_version").and_then(Value::as_u64))
            .unwrap_or_default();
        if same_day_version >= u64::from(HISTORICAL_MODEL_VERSION) {
            continue;
        }

        let previous = sqlx::query(
            "SELECT model_state::text AS state, observed_at FROM public.client_snapshots \
             WHERE account_id = $1 AND observed_on < $2 ORDER BY observed_on DESC LIMIT 1",
        )
        .bind(client.account_id)
        .bind(day)
        .fetch_optional(adapter.pool())
        .await?;
        let (mut model, previous_at, has_event_checkpoint) = match previous {
            Some(row) => {
                let state: String = row.get("state");
                let has_cursor = serde_json::from_str::<Value>(&state)?
                    .get("event_log_cursor")
                    .is_some();
                (
                    HistoricalModel::from_json(&state).context("invalid saved historical model")?,
                    Some(row.get::<DateTime<Utc>, _>("observed_at")),
                    has_cursor,
                )
            }
            None => (HistoricalModel::default(), None, false),
        };
        // Older saved states predate event-log ingestion. Start from now instead of
        // replaying initial seed/import rows as if they were a customer's behavior.
        if !has_event_checkpoint {
            model.set_event_log_cursor(event_highwater);
        }
        let mut event_assessments = Vec::new();
        if has_event_checkpoint {
            let logs = operation_logs_for_account(
                adapter,
                client.account_id,
                model.event_log_cursor(),
                event_highwater,
            )
            .await?;
            let mut events: Vec<_> = logs.iter().flat_map(events_from_log).collect();
            if let Some(previous_at) = previous_at {
                events.extend(
                    inferred_expiration_events(adapter, client.account_id, previous_at, now)
                        .await?,
                );
            }
            events.sort_by(|left, right| {
                left.occurred_at
                    .cmp(&right.occurred_at)
                    .then_with(|| left.order.cmp(&right.order))
            });
            for event in events {
                let assessment = model.observe_with_event(
                    event.occurred_at.timestamp_millis(),
                    &BTreeMap::new(),
                    Some(BehaviorEvent {
                        stream_id: &event.stream_id,
                        kind: &event.kind,
                    }),
                );
                event_assessments.push(assessment);
            }
        }
        model.set_event_log_cursor(event_highwater);
        let features = feature_map(&client);
        let mut assessment = model.observe(now.timestamp_millis(), &features, None);
        merge_event_assessments(&mut assessment, event_assessments);
        let inserted = sqlx::query(
            "INSERT INTO public.client_snapshots \
             (account_id, observed_on, observed_at, features, assessment, model_state) \
             VALUES ($1, $2, $3, $4::jsonb, $5::jsonb, $6::jsonb) \
             ON CONFLICT (account_id, observed_on) DO UPDATE SET \
                 observed_at = EXCLUDED.observed_at, features = EXCLUDED.features, \
                 assessment = EXCLUDED.assessment, model_state = EXCLUDED.model_state",
        )
        .bind(client.account_id)
        .bind(day)
        .bind(now)
        .bind(serde_json::to_string(&features)?)
        .bind(serde_json::to_string(&assessment)?)
        .bind(model.to_json()?)
        .execute(adapter.pool())
        .await?;
        count += inserted.rows_affected() as usize;
    }
    Ok(count)
}

fn feature_map(client: &ClientFeatures) -> BTreeMap<String, f64> {
    let mut values = BTreeMap::from([
        ("services_count".into(), client.services_count as f64),
        ("monthly_spend".into(), client.monthly_spend),
        ("spendable_balance".into(), client.spendable_balance as f64),
        ("upcoming_payments".into(), client.upcoming_payments as f64),
    ]);
    if let Some(coverage) = client.payment_coverage.filter(|value| value.is_finite()) {
        values.insert("payment_coverage".into(), coverage);
    }
    for (name, value) in [
        ("no_auto_prolong_share", client.no_auto_prolong_share),
        ("prolong_wait_share", client.prolong_wait_share),
        ("failed_prolong_share", client.failed_prolong_share),
        ("expiring_revenue_share", client.expiring_revenue_share),
    ] {
        if let Some(value) = value.filter(|value| value.is_finite()) {
            values.insert(name.into(), value);
        }
    }
    values.retain(|_, value| value.is_finite());
    values
}

#[derive(sqlx::FromRow)]
struct OperationLog {
    id: i64,
    occurred_at: DateTime<Utc>,
    entity: String,
    action: String,
    before: Option<Value>,
    after: Option<Value>,
    service_id: Option<i64>,
}

struct ObservedEvent {
    order: i64,
    occurred_at: DateTime<Utc>,
    stream_id: String,
    kind: String,
}

async fn operation_logs_for_account(
    adapter: &PgAdapter,
    account_id: i64,
    after_id: i64,
    through_id: i64,
) -> Result<Vec<OperationLog>> {
    Ok(sqlx::query_as::<_, OperationLog>(
        "SELECT l.id, l.\"occurredAt\" AS occurred_at, l.entity, l.action, l.before, l.after, \
         CASE WHEN l.entity = 'service' THEN l.\"entityId\" \
              WHEN l.entity = 'transaction' THEN COALESCE( \
                  (l.after->>'serviceId')::bigint, (l.before->>'serviceId')::bigint) \
              ELSE NULL END AS service_id \
         FROM public.operation_logs l \
         WHERE l.id > $2 AND l.id <= $3 AND l.\"occurredAt\" <= now() \
           AND (l.\"accountId\" = $1 \
             OR (l.entity = 'account' AND l.\"entityId\" = $1) \
             OR (l.entity = 'service' AND ( \
                    (l.after->>'ownerId')::bigint = $1 \
                    OR (l.before->>'ownerId')::bigint = $1)) \
             OR (l.entity = 'transaction' AND EXISTS ( \
                    SELECT 1 FROM public.services s \
                    WHERE s.id = COALESCE( \
                        (l.after->>'serviceId')::bigint, (l.before->>'serviceId')::bigint) \
                      AND s.\"ownerId\" = $1))) \
         ORDER BY l.\"occurredAt\", l.id",
    )
    .bind(account_id)
    .bind(after_id)
    .bind(through_id)
    .fetch_all(adapter.pool())
    .await?)
}

async fn inferred_expiration_events(
    adapter: &PgAdapter,
    account_id: i64,
    after: DateTime<Utc>,
    through: DateTime<Utc>,
) -> Result<Vec<ObservedEvent>> {
    let rows = sqlx::query(
        "SELECT id, \"expiresAt\" AS expires_at FROM public.services \
         WHERE \"ownerId\" = $1 AND status <> 'deleted' \
           AND \"createdAt\" <= $3 AND \"expiresAt\" > $2 AND \"expiresAt\" <= $3 \
         ORDER BY \"expiresAt\", id",
    )
    .bind(account_id)
    .bind(after)
    .bind(through)
    .fetch_all(adapter.pool())
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| ObservedEvent {
            order: i64::MAX,
            occurred_at: row.get("expires_at"),
            stream_id: format!("service:{}", row.get::<i64, _>("id")),
            kind: "expired".to_owned(),
        })
        .collect())
}

fn events_from_log(log: &OperationLog) -> Vec<ObservedEvent> {
    let mut events = Vec::new();
    match log.entity.as_str() {
        "service" => {
            let Some(service_id) = log.service_id else {
                return events;
            };
            let stream = format!("service:{service_id}");
            let before_status = json_string(&log.before, "status");
            let after_status = json_string(&log.after, "status");
            if log.action == "delete" {
                if json_datetime(&log.before, "expiresAt")
                    .is_some_and(|expires_at| expires_at <= log.occurred_at)
                {
                    push_event(&mut events, log, stream.clone(), "expired");
                }
                push_event(&mut events, log, stream.clone(), "deleted");
            } else if after_status != before_status {
                if after_status.as_deref() == Some("deleted")
                    && json_datetime(&log.after, "expiresAt")
                        .is_some_and(|expires_at| expires_at <= log.occurred_at)
                {
                    push_event(&mut events, log, stream.clone(), "expired");
                }
                if let Some(status) = after_status.as_deref() {
                    push_event(&mut events, log, stream.clone(), status);
                }
            }
            let before_auto = json_bool(&log.before, "autoProlong");
            let after_auto = json_bool(&log.after, "autoProlong");
            if before_auto != after_auto {
                match after_auto {
                    Some(true) => push_event(&mut events, log, stream.clone(), "auto_renew_on"),
                    Some(false) => push_event(&mut events, log, stream, "auto_renew_off"),
                    None => {}
                }
            }
        }
        "transaction" => {
            if json_string(&log.after, "type").as_deref() != Some("prolong") {
                return events;
            }
            let Some(service_id) = log.service_id else {
                return events;
            };
            let stream = format!("service:{service_id}");
            let before_status = json_string(&log.before, "status");
            let after_status = json_string(&log.after, "status");
            if log.action == "create" || before_status != after_status {
                match after_status.as_deref() {
                    Some("performed") => {
                        push_event(&mut events, log, stream.clone(), "prolong_succeeded")
                    }
                    Some("not_performed") => {
                        push_event(&mut events, log, stream, "prolong_outcome_unknown")
                    }
                    _ => {}
                }
            }
        }
        "account" => {
            let before_balance = json_i64(&log.before, "balance");
            let after_balance = json_i64(&log.after, "balance");
            let before_replenished = json_i64(&log.before, "totalReplenished");
            let after_replenished = json_i64(&log.after, "totalReplenished");
            let recorded_top_up = matches!((before_replenished, after_replenished),
                (Some(old), Some(new)) if new > old);
            if recorded_top_up {
                push_event(&mut events, log, "account".to_owned(), "top_up");
            }
            match (before_balance, after_balance) {
                (Some(before), Some(after)) if after > before && !recorded_top_up => push_event(
                    &mut events,
                    log,
                    "account".to_owned(),
                    "balance_credit_unclassified",
                ),
                (Some(before), Some(after)) if after < before => {
                    push_event(&mut events, log, "account".to_owned(), "balance_decreased")
                }
                _ => {}
            }
        }
        _ => {}
    }
    events
}

fn push_event(events: &mut Vec<ObservedEvent>, log: &OperationLog, stream_id: String, kind: &str) {
    events.push(ObservedEvent {
        order: log.id,
        occurred_at: log.occurred_at,
        stream_id,
        kind: kind.to_owned(),
    });
}

fn json_string(value: &Option<Value>, key: &str) -> Option<String> {
    value.as_ref()?.get(key)?.as_str().map(str::to_owned)
}

fn json_bool(value: &Option<Value>, key: &str) -> Option<bool> {
    value.as_ref()?.get(key)?.as_bool()
}

fn json_i64(value: &Option<Value>, key: &str) -> Option<i64> {
    value.as_ref()?.get(key)?.as_i64()
}

fn json_datetime(value: &Option<Value>, key: &str) -> Option<DateTime<Utc>> {
    value
        .as_ref()?
        .get(key)?
        .as_str()?
        .parse::<DateTime<Utc>>()
        .ok()
}

fn merge_event_assessments(snapshot: &mut HistoricalAssessment, events: Vec<HistoricalAssessment>) {
    for event in events {
        snapshot.transitions.extend(event.transitions);
        if event.transition.is_some() {
            snapshot.transition = event.transition;
        }
        snapshot.event_familiarity = snapshot.event_familiarity.max(event.event_familiarity);
        snapshot.is_confident |= event.is_confident;
        snapshot.is_unusual |= event.is_unusual;
        snapshot.anomaly_score = match (snapshot.anomaly_score, event.anomaly_score) {
            (Some(left), Some(right)) => Some(left.max(right)),
            (left, right) => left.or(right),
        };
        for matched in event.sequence_matches {
            if let Some(existing) = snapshot
                .sequence_matches
                .iter_mut()
                .find(|existing| existing.pattern == matched.pattern)
            {
                if matched.similarity > existing.similarity {
                    *existing = matched;
                }
            } else {
                snapshot.sequence_matches.push(matched);
            }
        }
        for deviation in event.personal_sequence_deviations {
            if let Some(existing) = snapshot
                .personal_sequence_deviations
                .iter_mut()
                .find(|existing| existing.stream_id == deviation.stream_id)
            {
                if deviation.started_at >= existing.started_at {
                    *existing = deviation;
                }
            } else {
                snapshot.personal_sequence_deviations.push(deviation);
            }
        }
        snapshot.anomaly_reasons.extend(event.anomaly_reasons);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn log(
        entity: &str,
        action: &str,
        before: Option<Value>,
        after: Option<Value>,
        service_id: Option<i64>,
    ) -> OperationLog {
        OperationLog {
            id: 1,
            occurred_at: DateTime::from_timestamp(1_800_000_000, 0).unwrap(),
            entity: entity.to_owned(),
            action: action.to_owned(),
            before,
            after,
            service_id,
        }
    }

    #[test]
    fn service_status_and_renewal_changes_become_ordered_events() {
        let row = log(
            "service",
            "update",
            Some(serde_json::json!({"status":"active", "autoProlong":true})),
            Some(serde_json::json!({"status":"prolong_wait", "autoProlong":false})),
            Some(42),
        );
        let events = events_from_log(&row);
        assert_eq!(
            events
                .iter()
                .map(|event| event.kind.as_str())
                .collect::<Vec<_>>(),
            vec!["prolong_wait", "auto_renew_off"]
        );
        assert!(events.iter().all(|event| event.stream_id == "service:42"));
    }

    #[test]
    fn unknown_prolong_outcome_and_balance_top_up_are_recorded_separately() {
        let prolong = log(
            "transaction",
            "create",
            None,
            Some(serde_json::json!({"type":"prolong", "status":"not_performed"})),
            Some(7),
        );
        let top_up = log(
            "account",
            "update",
            Some(serde_json::json!({"balance":100,"totalReplenished":100})),
            Some(serde_json::json!({"balance":250,"totalReplenished":250})),
            None,
        );
        let unclassified_credit = log(
            "account",
            "update",
            Some(serde_json::json!({"balance":100,"totalReplenished":100})),
            Some(serde_json::json!({"balance":250,"totalReplenished":100})),
            None,
        );
        let top_up_and_charge = log(
            "account",
            "update",
            Some(serde_json::json!({"balance":100,"totalReplenished":100})),
            Some(serde_json::json!({"balance":100,"totalReplenished":250})),
            None,
        );
        assert_eq!(events_from_log(&prolong)[0].kind, "prolong_outcome_unknown");
        assert_eq!(events_from_log(&top_up)[0].kind, "top_up");
        assert_eq!(events_from_log(&top_up_and_charge)[0].kind, "top_up");
        assert_eq!(
            events_from_log(&unclassified_credit)[0].kind,
            "balance_credit_unclassified"
        );
    }
}
