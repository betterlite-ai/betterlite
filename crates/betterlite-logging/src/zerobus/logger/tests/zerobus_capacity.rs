use std::sync::Arc;

use crate::callback_logger::CallbackLogger;
use crate::zerobus::logger::ZerobusLogger;
use crate::zerobus::logger::delivery::RowIngest;

use super::support::{
    Buffer, FakeIngest, deps, event, expected_ids, params, queued_ids, settle, subscribe, until,
};

#[tokio::test]
async fn zerobus_capacity_drops_the_arriving_row_at_the_cap_during_a_flush() {
    let ingest = FakeIngest::table("main.litellm.traces");
    let gate = ingest.block();
    let client: Arc<dyn RowIngest> = Arc::clone(&ingest);
    let mut configured = deps(params(2, 3600, false), Some(client));
    configured.max_queue_size = Some(3);
    let logger = ZerobusLogger::new(configured).expect("the logger builds");
    let buffer = Buffer::default();
    let guard = subscribe(&buffer);
    logger
        .async_log_success_event(event("a"))
        .await
        .expect("the event is accepted");
    logger
        .async_log_success_event(event("b"))
        .await
        .expect("the event is accepted");
    assert!(
        until(|| ingest.inserted() == 1).await,
        "the batch flush starts"
    );
    logger
        .async_log_success_event(event("c"))
        .await
        .expect("the event is accepted");
    logger
        .async_log_success_event(event("d"))
        .await
        .expect("the event is accepted");
    logger
        .async_log_success_event(event("e"))
        .await
        .expect("the event is accepted");
    assert_eq!(queued_ids(&logger), expected_ids(&["c"]));
    ingest.unblock();
    gate.add_permits(1);
    settle(&logger).await;
    drop(guard);
    assert_eq!(ingest.inserted(), 1);
    assert_eq!(ingest.ids(), expected_ids(&["a", "b"]));
    assert!(
        buffer.contents().contains("dropped a row"),
        "{}",
        buffer.contents()
    );
}

#[tokio::test]
async fn zerobus_capacity_queues_an_arrival_below_the_cap_during_a_flush() {
    let ingest = FakeIngest::table("main.litellm.traces");
    let gate = ingest.block();
    let client: Arc<dyn RowIngest> = Arc::clone(&ingest);
    let mut configured = deps(params(1, 3600, false), Some(client));
    configured.max_queue_size = Some(3);
    let logger = ZerobusLogger::new(configured).expect("the logger builds");
    logger
        .async_log_success_event(event("a"))
        .await
        .expect("the event is accepted");
    assert!(
        until(|| ingest.inserted() == 1).await,
        "the batch flush starts"
    );
    logger
        .async_log_success_event(event("b"))
        .await
        .expect("the event is accepted");
    assert_eq!(queued_ids(&logger), expected_ids(&["b"]));
    ingest.unblock();
    gate.add_permits(1);
    settle(&logger).await;
    assert_eq!(queued_ids(&logger), expected_ids(&["b"]));
}

#[tokio::test]
async fn zerobus_capacity_queues_again_after_the_flush() {
    let ingest = FakeIngest::table("main.litellm.traces");
    let gate = ingest.block();
    let client: Arc<dyn RowIngest> = Arc::clone(&ingest);
    let mut configured = deps(params(2, 3600, false), Some(client));
    configured.max_queue_size = Some(3);
    let logger = ZerobusLogger::new(configured).expect("the logger builds");
    logger
        .async_log_success_event(event("a"))
        .await
        .expect("the event is accepted");
    logger
        .async_log_success_event(event("b"))
        .await
        .expect("the event is accepted");
    assert!(
        until(|| ingest.inserted() == 1).await,
        "the batch flush starts"
    );
    logger
        .async_log_success_event(event("c"))
        .await
        .expect("the event is accepted");
    logger
        .async_log_success_event(event("d"))
        .await
        .expect("the event is accepted");
    assert_eq!(queued_ids(&logger), expected_ids(&["c"]));
    ingest.unblock();
    gate.add_permits(1);
    settle(&logger).await;
    logger
        .async_log_success_event(event("e"))
        .await
        .expect("the event is accepted");
    settle(&logger).await;
    assert_eq!(ingest.inserted(), 2);
    assert_eq!(ingest.ids(), expected_ids(&["a", "b", "c", "e"]));
    assert!(queued_ids(&logger).is_empty());
}
