use std::sync::Arc;

use crate::callback_logger::CallbackLogger as _;
use crate::zerobus::logger::CALLBACK_NAME;
use crate::zerobus::logger::delivery::RowIngest;

use super::support::{FakeIngest, event, expected_ids, logger, params, queued_ids};

#[tokio::test]
async fn callback_logger_names_the_zerobus_adapter() {
    let client: Arc<dyn RowIngest> = FakeIngest::table("main.litellm.traces");
    let logger = logger(params(100, 3600, false), client);
    assert_eq!(logger.callback_name(), CALLBACK_NAME);
}

#[tokio::test]
async fn callback_logger_reports_the_message_logging_setting() {
    let client: Arc<dyn RowIngest> = FakeIngest::table("main.litellm.traces");
    let redacting = logger(params(100, 3600, true), client);
    assert!(redacting.turn_off_message_logging());
    let plain_client: Arc<dyn RowIngest> = FakeIngest::table("main.litellm.traces");
    let plain = logger(params(100, 3600, false), plain_client);
    assert!(!plain.turn_off_message_logging());
}

#[tokio::test]
async fn callback_logger_success_and_failure_events_reach_the_queue() {
    let client: Arc<dyn RowIngest> = FakeIngest::table("main.litellm.traces");
    let logger = logger(params(100, 3600, false), client);
    logger
        .async_log_success_event(event("chatcmpl-ok"))
        .await
        .expect("the success event is accepted");
    logger
        .async_log_failure_event(event("chatcmpl-err"))
        .await
        .expect("the failure event is accepted");
    assert_eq!(
        queued_ids(&logger),
        expected_ids(&["chatcmpl-ok", "chatcmpl-err"])
    );
}

#[tokio::test]
async fn callback_logger_ignores_stream_events() {
    let client: Arc<dyn RowIngest> = FakeIngest::table("main.litellm.traces");
    let logger = logger(params(100, 3600, false), client);
    logger
        .async_log_stream_event(event("chatcmpl-stream"))
        .await
        .expect("the stream event is accepted");
    assert!(queued_ids(&logger).is_empty());
}
