#![cfg(test)]

use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::callback_logger::CallbackLogger as _;
use crate::gcs_logger::gcs_bucket_logging::CALLBACK_NAME;

pub(crate) mod support;

use support::{Answer, event, open};

#[tokio::test]
async fn callback_logger_is_named_gcs_bucket() {
    let harness = open(&[], Some("bucket"), Answer::ok("{}"));
    assert_eq!(harness.logger.callback_name(), CALLBACK_NAME);
    assert_eq!(harness.logger.callback_name(), "gcs_bucket");
}

#[tokio::test]
async fn callback_logger_queues_a_success_event_without_uploading() {
    let harness = open(&[], Some("bucket"), Answer::ok("{}"));
    harness
        .logger
        .async_log_success_event(event("chatcmpl-1", None, Json::Null, IndexMap::new()))
        .await
        .expect("the success event is accepted");
    let queued = harness.queued();
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].payload.id, "chatcmpl-1");
    assert_eq!(queued[0].response_id, "chatcmpl-1");
    assert!(harness.requests().is_empty());
}

#[tokio::test]
async fn callback_logger_queues_a_failure_event_without_uploading() {
    let harness = open(&[], Some("bucket"), Answer::ok("{}"));
    harness
        .logger
        .async_log_failure_event(event(
            "chatcmpl-2",
            Some("the upstream call failed"),
            Json::Null,
            IndexMap::new(),
        ))
        .await
        .expect("the failure event is accepted");
    let queued = harness.queued();
    assert_eq!(queued.len(), 1);
    assert_eq!(
        queued[0].payload.error_str.as_deref(),
        Some("the upstream call failed")
    );
    assert!(harness.requests().is_empty());
}

#[tokio::test]
async fn callback_logger_accepts_events_when_the_upload_fails() {
    let harness = open(
        &[
            ("LITELLM_ASYNCIO_QUEUE_MAXSIZE", "1"),
            ("GCS_BATCH_SIZE", "1"),
        ],
        Some("bucket"),
        Answer::with_status(500),
    );
    let success = harness
        .logger
        .async_log_success_event(event("chatcmpl-1", None, Json::Null, IndexMap::new()))
        .await;
    success.expect("the callback accepts the success event");
    let failure = harness
        .logger
        .async_log_failure_event(event(
            "chatcmpl-2",
            Some("the upstream call failed"),
            Json::Null,
            IndexMap::new(),
        ))
        .await;
    failure.expect("the callback accepts the failure event");
    assert_eq!(harness.requests().len(), 1);
    assert_eq!(harness.queue_len(), 1);
}
