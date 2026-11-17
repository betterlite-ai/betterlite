use std::sync::Arc;

use crate::callback_logger::CallbackLogger as _;
use crate::zerobus::logger::ZerobusLogger;
use crate::zerobus::logger::delivery::RowIngest;

use super::support::{
    Buffer, FakeIngest, as_row_ingest, deps, event, expected_ids, params, queued_ids, settle,
    subscribe, until,
};

#[tokio::test]
async fn zerobus_overflow_trims_the_oldest_rows_between_flushes() {
    let ingest = FakeIngest::table("main.litellm.traces");
    let client: Arc<dyn RowIngest> = as_row_ingest(&ingest);
    let mut configured = deps(params(10, 3600, false), Some(client));
    configured.max_queue_size = Some(2);
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
    logger
        .async_log_success_event(event("c"))
        .await
        .expect("the event is accepted");
    drop(guard);
    assert_eq!(queued_ids(&logger), expected_ids(&["b", "c"]));
    assert_eq!(ingest.inserted(), 0);
    assert!(
        buffer.contents().contains("dropped the oldest events"),
        "{}",
        buffer.contents()
    );
}

#[tokio::test]
async fn zerobus_overflow_keeps_rows_that_arrive_during_a_flush() {
    let ingest = FakeIngest::table("main.litellm.traces");
    let gate = ingest.block();
    let client: Arc<dyn RowIngest> = as_row_ingest(&ingest);
    let mut configured = deps(params(1, 3600, false), Some(client));
    configured.max_queue_size = Some(2);
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
