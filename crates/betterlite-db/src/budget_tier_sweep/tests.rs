#![cfg(test)]

pub(crate) mod support;

use crate::budget_reset_policy::phase_draining::ResetLimits;
use crate::budget_tier_sweep::tests::support::{
    CountingReconnect, Harness, RecordingCache, TestSchedule, finish, insert_tier, moment, naive,
    past, reset_at_of, schema, unreset_count,
};

#[tokio::test]
async fn phase_stops_at_the_chunk_cap() {
    let (fixture, store, client) = schema().await;
    insert_tier(&client, "t1", Some("1d"), Some(past())).await;
    insert_tier(&client, "t2", Some("1d"), Some(past())).await;
    insert_tier(&client, "t3", Some("1d"), Some(past())).await;
    insert_tier(&client, "t4", Some("1d"), Some(past())).await;
    insert_tier(&client, "t5", Some("1d"), Some(past())).await;
    insert_tier(&client, "t6", Some("1d"), Some(past())).await;
    let mut harness = Harness::new(
        store.clone(),
        TestSchedule::fixed(moment(30)),
        RecordingCache::default(),
        CountingReconnect::new(true),
    );
    harness.limits = ResetLimits {
        batch_size: 2,
        max_chunks_per_run: 2,
    };
    harness.run().await;
    assert_eq!(harness.successes.events().len(), 2);
    assert_eq!(unreset_count(&client).await, 2);
    finish(fixture, store, client).await;
}

#[tokio::test]
async fn a_chunk_that_advances_nothing_drains_the_phase() {
    let (fixture, store, client) = schema().await;
    insert_tier(&client, "t1", Some("1d"), Some(past())).await;
    insert_tier(&client, "t2", Some("1d"), Some(past())).await;
    let mut harness = Harness::new(
        store.clone(),
        TestSchedule::fixed(moment(0)),
        RecordingCache::default(),
        CountingReconnect::new(true),
    );
    harness.limits = ResetLimits {
        batch_size: 2,
        max_chunks_per_run: 100,
    };
    harness.run().await;
    assert_eq!(harness.successes.events().len(), 1);
    assert_eq!(reset_at_of(&client, "t1").await, Some(naive(0)));
    finish(fixture, store, client).await;
}
