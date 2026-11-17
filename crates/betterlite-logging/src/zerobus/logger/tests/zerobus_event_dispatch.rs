use std::sync::Arc;

use crate::callback_logger::CallbackLogger as _;
use crate::zerobus::logger::delivery::RowIngest;

use super::support::{FakeIngest, event, logger, params, queued_ids, queued_rows};

#[tokio::test]
async fn zerobus_event_dispatch_sends_success_and_failure_alike() {
    let client: Arc<dyn RowIngest> = FakeIngest::table("main.litellm.traces");
    let logger = logger(params(100, 3600, false), client);
    logger
        .async_log_success_event(event("chatcmpl-x"))
        .await
        .expect("the success event is accepted");
    logger
        .async_log_failure_event(event("chatcmpl-x"))
        .await
        .expect("the failure event is accepted");
    let rows = queued_rows(&logger);
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0], rows[1]);
}

#[tokio::test]
async fn zerobus_event_dispatch_stream_events_never_reach_the_queue() {
    let client: Arc<dyn RowIngest> = FakeIngest::table("main.litellm.traces");
    let logger = logger(params(100, 3600, false), client);
    logger
        .async_log_stream_event(event("chatcmpl-s"))
        .await
        .expect("the stream event is accepted");
    assert!(queued_ids(&logger).is_empty());
}
