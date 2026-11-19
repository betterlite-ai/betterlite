#![cfg(test)]

use betterlite_models::decisions::Json;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde_json::Number;

use super::collect_resets;
use crate::budget_reset_policy::cache_invalidation::InvalidatedCache;
use crate::budget_reset_policy::phase_draining::ResetLimits;
use crate::budget_reset_policy::rollover::RolloverPolicy;
use crate::identity_budget_reset::phase::IdentityPhase;
use crate::identity_budget_reset::rows::DueIdentity;
use crate::identity_budget_reset::tests::support::{
    CountingReconnect, Harness, RecordingCache, TestSchedule, assert_spend, finish, moment, naive,
    past, reset_at_of, schema,
};

fn identity(identifier: &str, budget_duration: &str) -> DueIdentity {
    DueIdentity {
        identifier: identifier.to_owned(),
        spend: Decimal::from(10_i32),
        max_budget: Some(Decimal::from(5_i32)),
        budget_duration: budget_duration.to_owned(),
        budget_reset_at: Some(moment(1)),
    }
}

fn next() -> DateTime<Utc> {
    moment(30)
}

#[test]
fn per_row_failure_keeps_the_rows_that_reset() {
    let schedule = TestSchedule::failing_for(next(), "bad", "no schedule");
    let rows = vec![
        identity("a", "1d"),
        identity("b", "bad"),
        identity("c", "1d"),
    ];
    let (writes, failures) = collect_resets(
        IdentityPhase::Keys,
        &rows,
        RolloverPolicy::default(),
        &schedule,
    );
    assert_eq!(
        writes
            .iter()
            .map(|write| write.identifier.clone())
            .collect::<Vec<String>>(),
        vec!["a".to_owned(), "c".to_owned()]
    );
    assert_eq!(failures.rows.len(), 1);
    assert_eq!(failures.rows[0].row, "b");
    assert_eq!(failures.rows[0].error, "no schedule");
}

#[test]
fn per_row_failure_summary_counts_and_names_every_row() {
    let schedule = TestSchedule::failing_for(next(), "bad", "no schedule");
    let rows = vec![
        identity("a", "bad"),
        identity("b", "1d"),
        identity("c", "bad"),
    ];
    let (_writes, failures) = collect_resets(
        IdentityPhase::Keys,
        &rows,
        RolloverPolicy::default(),
        &schedule,
    );
    assert_eq!(
        failures.summary(IdentityPhase::Keys.subject()),
        Some("Failed to reset 2 keys: a: no schedule; c: no schedule".to_owned())
    );
    assert_eq!(
        failures.summary(IdentityPhase::Users.subject()),
        Some("Failed to reset 2 users: a: no schedule; c: no schedule".to_owned())
    );
}

#[tokio::test]
async fn per_row_failure_commits_the_good_rows_and_reports_the_chunk() {
    let (fixture, store, client) = schema().await;
    client
        .batch_execute(
            "INSERT INTO \"LiteLLM_VerificationToken\" (token, spend, budget_duration, budget_reset_at) VALUES ('good', 10, '1d', '2025-12-30 00:00:00'), ('bad', 10, 'bad', '2025-12-30 00:00:00');",
        )
        .await
        .unwrap();
    let schedule = TestSchedule::failing_for(next(), "bad", "no schedule");
    let mut harness = Harness::new(
        store.clone(),
        schedule,
        RecordingCache::default(),
        CountingReconnect::new(true),
    );
    harness.run(IdentityPhase::Keys).await;
    assert_spend(&client, "LiteLLM_VerificationToken", "token", "good", 0.0).await;
    assert_spend(&client, "LiteLLM_VerificationToken", "token", "bad", 10.0).await;
    assert_eq!(
        reset_at_of(&client, "LiteLLM_VerificationToken", "token", "good").await,
        Some(naive(30))
    );
    assert_eq!(
        reset_at_of(&client, "LiteLLM_VerificationToken", "token", "bad").await,
        Some(past())
    );
    let calls = harness.cache.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].sink, InvalidatedCache::SpendCounter);
    assert_eq!(calls[0].keys, vec!["spend:key:good".to_owned()]);
    assert!(harness.successes.events().is_empty());
    let events = harness.failures.events();
    assert_eq!(events.len(), 1);
    let event = &events[0];
    assert_eq!(event.call_type, "reset_budget_keys");
    assert_eq!(event.error, "Failed to reset 1 keys: bad: no schedule");
    assert_eq!(
        event.event_metadata.get("num_keys_found"),
        Some(&Json::Number(Number::from(2_u8)))
    );
    finish(fixture, store, client).await;
}

#[tokio::test]
async fn per_row_failure_keeps_a_full_chunk_advancing_the_phase() {
    let (fixture, store, client) = schema().await;
    client
        .batch_execute(
            "INSERT INTO \"LiteLLM_VerificationToken\" (token, spend, budget_duration, budget_reset_at) VALUES ('good', 10, '1d', '2025-12-30 00:00:00'), ('bad', 10, 'bad', '2025-12-30 00:00:00');",
        )
        .await
        .unwrap();
    let schedule = TestSchedule::failing_for(next(), "bad", "no schedule");
    let mut harness = Harness::new(
        store.clone(),
        schedule,
        RecordingCache::default(),
        CountingReconnect::new(true),
    );
    harness.limits = ResetLimits {
        batch_size: 2,
        max_chunks_per_run: 2,
    };
    harness.run(IdentityPhase::Keys).await;
    // The first chunk filled the batch, so the phase read again and the failing row failed a second time.
    assert_spend(&client, "LiteLLM_VerificationToken", "token", "good", 0.0).await;
    assert_eq!(harness.cache.calls().len(), 1);
    assert!(harness.successes.events().is_empty());
    assert_eq!(harness.failures.events().len(), 2);
    finish(fixture, store, client).await;
}
