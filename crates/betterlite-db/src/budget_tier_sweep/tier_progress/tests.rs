#![cfg(test)]

use betterlite_models::decisions::Json;
use serde_json::Number;

use crate::budget_reset_policy::phase_draining::ResetLimits;
use crate::budget_tier_sweep::enduser_page_truncation::EndUserWalk;
use crate::budget_tier_sweep::tests::support::{
    CountingReconnect, Harness, RecordingCache, TestClock, TestSchedule, finish, insert_tier,
    moment, past, schema,
};
use crate::budget_tier_sweep::tier_progress::success_metadata;

#[test]
fn success_metadata_names_every_count_and_the_truncation_flag() {
    let walk = EndUserWalk {
        cursor: None,
        invalidated: 2,
        truncated: true,
    };
    let metadata = success_metadata(3, &walk);
    assert_eq!(
        metadata["num_budgets_found"],
        Json::Number(Number::from(3_u8))
    );
    assert_eq!(
        metadata["num_endusers_found"],
        Json::Number(Number::from(2_u8))
    );
    assert_eq!(
        metadata["num_endusers_updated"],
        Json::Number(Number::from(2_u8))
    );
    assert_eq!(
        metadata["num_endusers_failed"],
        Json::Number(Number::from(0_u8))
    );
    assert_eq!(metadata["enduser_invalidation_truncated"], Json::Bool(true));
}

#[tokio::test]
async fn chunk_success_reports_counts_timing_and_no_truncation() {
    let (fixture, store, client) = schema().await;
    insert_tier(&client, "tier-a", Some("1d"), Some(past())).await;
    client
        .batch_execute(
            "INSERT INTO \"LiteLLM_EndUserTable\" (user_id, budget_id, spend) VALUES ('customer-a', 'tier-a', 10)",
        )
        .await
        .unwrap();
    let mut harness = Harness::new(
        store.clone(),
        TestSchedule::fixed(moment(30)),
        RecordingCache::default(),
        CountingReconnect::new(true),
    );
    harness.clock = TestClock::scripted(vec![moment(0), moment(4), moment(10)], moment(0));
    harness.run().await;
    let events = harness.successes.events();
    assert_eq!(events.len(), 1);
    let event = &events[0];
    assert_eq!(event.service, "reset_budget_job");
    assert_eq!(event.call_type, "reset_budget_budget_table");
    assert_eq!(event.start_time, moment(0));
    assert_eq!(event.end_time, moment(10));
    assert_eq!(event.duration_seconds.to_bits(), 10.0_f64.to_bits());
    assert_eq!(
        event.event_metadata["num_budgets_found"],
        Json::Number(Number::from(1_u8))
    );
    assert_eq!(
        event.event_metadata["num_endusers_found"],
        Json::Number(Number::from(1_u8))
    );
    assert_eq!(
        event.event_metadata["enduser_invalidation_truncated"],
        Json::Bool(false)
    );
    assert!(harness.failures.events().is_empty());
    finish(fixture, store, client).await;
}

#[tokio::test]
async fn empty_chunk_still_reports_a_success_with_zero_counts() {
    let (fixture, store, client) = schema().await;
    let mut harness = Harness::new(
        store.clone(),
        TestSchedule::fixed(moment(30)),
        RecordingCache::default(),
        CountingReconnect::new(true),
    );
    harness.run().await;
    let events = harness.successes.events();
    assert_eq!(events.len(), 1);
    assert_eq!(
        events[0].event_metadata["num_budgets_found"],
        Json::Number(Number::from(0_u8))
    );
    assert_eq!(
        events[0].event_metadata["num_endusers_found"],
        Json::Number(Number::from(0_u8))
    );
    assert!(harness.failures.events().is_empty());
    finish(fixture, store, client).await;
}

#[tokio::test]
async fn truncated_walk_reports_its_reached_count_and_flag() {
    let (fixture, store, client) = schema().await;
    insert_tier(&client, "tier-a", Some("1d"), Some(past())).await;
    client
        .batch_execute(
            "INSERT INTO \"LiteLLM_EndUserTable\" (user_id, budget_id, spend) VALUES ('customer-a', 'tier-a', 10), ('customer-b', 'tier-a', 10)",
        )
        .await
        .unwrap();
    let cache = RecordingCache::sabotaging(store.clone());
    let mut harness = Harness::new(
        store.clone(),
        TestSchedule::fixed(moment(30)),
        cache,
        CountingReconnect::new(true),
    );
    harness.limits = ResetLimits {
        batch_size: 1,
        max_chunks_per_run: 1,
    };
    harness.run().await;
    let events = harness.successes.events();
    assert_eq!(events.len(), 1);
    assert_eq!(
        events[0].event_metadata["num_endusers_found"],
        Json::Number(Number::from(1_u8))
    );
    assert_eq!(
        events[0].event_metadata["num_endusers_updated"],
        Json::Number(Number::from(1_u8))
    );
    assert_eq!(
        events[0].event_metadata["enduser_invalidation_truncated"],
        Json::Bool(true)
    );
    finish(fixture, store, client).await;
}
