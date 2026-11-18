#![cfg(test)]

use super::progress;
use crate::budget_reset_policy::chunk_outcome::ChunkOutcome;
use crate::budget_reset_policy::phase_draining::{ResetLimits, phase_is_drained};
use crate::identity_budget_reset::phase::IdentityPhase;
use crate::identity_budget_reset::tests::support::{
    CountingReconnect, Harness, RecordingCache, TestClock, TestSchedule, finish, moment, naive,
    reset_at_of, reset_spend_count, schema,
};

const DUE_KEYS: &str = "INSERT INTO \"LiteLLM_VerificationToken\" (token, spend, budget_duration, budget_reset_at) VALUES ('a', 10, '1d', '2025-12-30 00:00:00'), ('b', 10, '1d', '2025-12-30 00:00:00'), ('c', 10, '1d', '2025-12-30 00:00:00');";

#[test]
fn chunk_outcome_counts_only_reset_times_past_the_cutoff() {
    let outcome = progress(3, vec![Some(moment(9)), Some(moment(1)), None], moment(5));
    assert_eq!(
        outcome,
        ChunkOutcome {
            fetched: 3,
            advanced: 1
        }
    );
}

#[test]
fn chunk_outcome_does_not_count_a_reset_time_equal_to_the_cutoff() {
    let outcome = progress(1, vec![Some(moment(5))], moment(5));
    assert_eq!(outcome.advanced, 0);
}

#[test]
fn chunk_outcome_drains_the_phase_when_a_full_chunk_advanced_nothing() {
    let stalled = progress(2, vec![Some(moment(1)), None], moment(5));
    assert!(phase_is_drained(stalled, 2));
    let advanced = progress(2, vec![Some(moment(9)), None], moment(5));
    assert!(!phase_is_drained(advanced, 2));
    let short = progress(1, vec![Some(moment(9))], moment(5));
    assert!(phase_is_drained(short, 2));
}

#[tokio::test]
async fn chunk_outcome_keeps_reading_full_chunks_that_advance() {
    let (fixture, store, client) = schema().await;
    client
        .batch_execute(
            "INSERT INTO \"LiteLLM_VerificationToken\" (token, spend, budget_duration, budget_reset_at) VALUES ('a', 10, '1d', '2025-12-30 00:00:00'), ('b', 10, '1d', '2025-12-30 00:00:00');",
        )
        .await
        .unwrap();
    let mut harness = Harness::new(
        store.clone(),
        TestSchedule::fixed(moment(30)),
        RecordingCache::default(),
        CountingReconnect::new(true),
    );
    harness.limits = ResetLimits {
        batch_size: 1,
        max_chunks_per_run: 5,
    };
    harness.run(IdentityPhase::Keys).await;
    assert_eq!(reset_spend_count(&client).await, 2);
    assert_eq!(harness.clock.calls(), 9);
    assert_eq!(harness.successes.events().len(), 3);
    finish(fixture, store, client).await;
}

#[tokio::test]
async fn chunk_outcome_stops_at_max_chunks_per_run() {
    let (fixture, store, client) = schema().await;
    client.batch_execute(DUE_KEYS).await.unwrap();
    let mut harness = Harness::new(
        store.clone(),
        TestSchedule::fixed(moment(30)),
        RecordingCache::default(),
        CountingReconnect::new(true),
    );
    harness.limits = ResetLimits {
        batch_size: 1,
        max_chunks_per_run: 2,
    };
    harness.run(IdentityPhase::Keys).await;
    assert_eq!(reset_spend_count(&client).await, 2);
    assert_eq!(harness.clock.calls(), 6);
    assert_eq!(harness.successes.events().len(), 2);
    finish(fixture, store, client).await;
}

#[tokio::test]
async fn chunk_outcome_stops_when_a_full_chunk_advanced_nothing() {
    let (fixture, store, client) = schema().await;
    client.batch_execute(DUE_KEYS).await.unwrap();
    let mut harness = Harness::new(
        store.clone(),
        TestSchedule::fixed(moment(0)),
        RecordingCache::default(),
        CountingReconnect::new(true),
    );
    harness.limits = ResetLimits {
        batch_size: 1,
        max_chunks_per_run: 2,
    };
    harness.run(IdentityPhase::Keys).await;
    assert_eq!(reset_spend_count(&client).await, 1);
    assert_eq!(harness.clock.calls(), 3);
    assert_eq!(harness.successes.events().len(), 1);
    finish(fixture, store, client).await;
}

#[tokio::test]
async fn chunk_outcome_reads_the_cutoff_after_the_chunk_finished() {
    let (fixture, store, client) = schema().await;
    client
        .batch_execute(
            "INSERT INTO \"LiteLLM_VerificationToken\" (token, spend, budget_duration, budget_reset_at) VALUES ('a', 10, '1d', '2025-12-30 00:00:00');",
        )
        .await
        .unwrap();
    let mut harness = Harness::new(
        store.clone(),
        TestSchedule::fixed(moment(5)),
        RecordingCache::default(),
        CountingReconnect::new(true),
    );
    harness.limits = ResetLimits {
        batch_size: 1,
        max_chunks_per_run: 2,
    };
    harness.clock = TestClock::scripted(vec![moment(1), moment(10), moment(2)], moment(3));
    harness.run(IdentityPhase::Keys).await;
    assert_eq!(harness.clock.calls(), 6);
    assert_eq!(harness.successes.events().len(), 2);
    assert_eq!(
        reset_at_of(&client, "LiteLLM_VerificationToken", "token", "a").await,
        Some(naive(5))
    );
    finish(fixture, store, client).await;
}
