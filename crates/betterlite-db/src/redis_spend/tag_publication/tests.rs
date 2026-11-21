#![cfg(test)]

use std::sync::Arc;

use crate::mcp_stored_contracts::MAX_STORED_BYTES;
use crate::redis_spend::codec::decode_daily_batch;
use crate::redis_spend::events::RedisBufferEvent;
use crate::redis_spend::keys::{REDIS_DAILY_TAG_BUFFER_KEY, SpendBufferQueue};
use crate::redis_spend::tag_publication::{TagPublishError, store_daily_tag_spend_updates};
use crate::redis_spend::tests::support::{
    FakeLists, captured_logs, daily_queue, daily_row, detached_publisher, publisher,
    publisher_recording,
};

mod real_redis;

#[tokio::test]
async fn the_tag_queue_is_drained_even_without_a_redis_client() {
    let publisher = detached_publisher();
    let queue = daily_queue(32, 16, 32);
    queue
        .add_update(daily_row("day-1", "1.5", 1))
        .await
        .unwrap();
    store_daily_tag_spend_updates(&publisher, &queue)
        .await
        .unwrap();
    assert!(
        queue
            .flush_aggregated_daily_rows()
            .await
            .unwrap()
            .is_empty(),
        "the drained tag aggregate is lost when no client can take it"
    );
}

#[tokio::test]
async fn a_tag_cohort_is_pushed_as_one_value_and_gauged() {
    let (logs, _guard) = captured_logs();
    let lists = Arc::new(FakeLists::default());
    let (publisher, events) = publisher_recording(&lists);
    lists.single_length(4);
    let queue = daily_queue(32, 16, 32);
    queue
        .add_update(daily_row("day-1", "1.5", 1))
        .await
        .unwrap();
    store_daily_tag_spend_updates(&publisher, &queue)
        .await
        .unwrap();
    let pushes = lists.single_pushes();
    assert_eq!(pushes.len(), 1);
    assert_eq!(pushes[0].0, REDIS_DAILY_TAG_BUFFER_KEY);
    assert_eq!(pushes[0].1.len(), 1);
    assert_eq!(
        decode_daily_batch(pushes[0].1[0].as_bytes()).unwrap(),
        daily_row("day-1", "1.5", 1)
    );
    assert_eq!(
        events.events(),
        vec![RedisBufferEvent {
            queue: SpendBufferQueue::DailyTag,
            queue_size: 4,
        }]
    );
    assert!(
        logs.text()
            .contains("redis_key=litellm_daily_tag_spend_update_buffer, buffer_size=4")
    );
}

#[tokio::test]
async fn a_failed_tag_push_is_logged_without_restoring() {
    let (logs, _guard) = captured_logs();
    let lists = Arc::new(FakeLists::default());
    let publisher = publisher(&lists);
    lists.fail_push();
    let queue = daily_queue(32, 16, 32);
    queue
        .add_update(daily_row("day-1", "1.5", 1))
        .await
        .unwrap();
    store_daily_tag_spend_updates(&publisher, &queue)
        .await
        .unwrap();
    let text = logs.text();
    assert!(text.contains(
        "failed to push spend updates to Redis (redis_key=litellm_daily_tag_spend_update_buffer)"
    ));
    assert!(!text.contains("Restoring"));
    assert!(
        queue
            .flush_aggregated_daily_rows()
            .await
            .unwrap()
            .is_empty(),
        "the tag path logs the failure and keeps no copy of the drained aggregate"
    );
}

#[tokio::test]
async fn an_empty_tag_queue_pushes_nothing() {
    let lists = Arc::new(FakeLists::default());
    let (publisher, events) = publisher_recording(&lists);
    let queue = daily_queue(32, 16, 32);
    store_daily_tag_spend_updates(&publisher, &queue)
        .await
        .unwrap();
    assert!(lists.single_pushes().is_empty());
    assert!(events.events().is_empty());
}

#[tokio::test]
async fn tag_encoding_runs_before_the_client_check() {
    let publisher = detached_publisher();
    let oversized = "x".repeat(MAX_STORED_BYTES + 1);
    let queue = daily_queue(32, 16, 32);
    queue
        .add_update(daily_row(&oversized, "1.5", 1))
        .await
        .unwrap();
    let error = store_daily_tag_spend_updates(&publisher, &queue)
        .await
        .unwrap_err();
    assert!(matches!(error, TagPublishError::Encode(_)), "{error:?}");
}
