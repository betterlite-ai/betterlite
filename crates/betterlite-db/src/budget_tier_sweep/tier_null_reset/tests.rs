#![cfg(test)]

use crate::budget_tier_sweep::tests::support::{
    CountingReconnect, Harness, RecordingCache, TestSchedule, finish, insert_tier, moment, naive,
    reset_at_of, schema,
};

#[tokio::test]
async fn a_tier_with_no_reset_timestamp_is_due_and_resets() {
    let (fixture, store, client) = schema().await;
    insert_tier(&client, "never-reset", Some("1d"), None).await;
    let mut harness = Harness::new(
        store.clone(),
        TestSchedule::fixed(moment(30)),
        RecordingCache::default(),
        CountingReconnect::new(true),
    );
    harness.run().await;
    assert_eq!(reset_at_of(&client, "never-reset").await, Some(naive(30)));
    finish(fixture, store, client).await;
}

#[tokio::test]
async fn a_tier_without_a_duration_stays_out_of_the_phase() {
    let (fixture, store, client) = schema().await;
    insert_tier(&client, "no-duration", None, None).await;
    let mut harness = Harness::new(
        store.clone(),
        TestSchedule::fixed(moment(30)),
        RecordingCache::default(),
        CountingReconnect::new(true),
    );
    harness.run().await;
    assert_eq!(reset_at_of(&client, "no-duration").await, None);
    assert!(harness.schedule.durations().is_empty());
    finish(fixture, store, client).await;
}
