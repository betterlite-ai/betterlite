use crate::callback_logger::CallbackLogger as _;
use crate::fields::payload::StandardLoggingPayload;
use crate::fields::status::LoggingStatus;

use super::support::{Harness, event, payload, payload_with_messages, settings_from_environment};

const BASE64_MESSAGES: &str = r#"[{"role":"user","content":[{"type":"text","text":"data:image/png;base64,iVBORw0KGgo="},{"type":"text","text":"hello"}]}]"#;

fn payload_with_base64() -> StandardLoggingPayload {
    payload_with_messages(BASE64_MESSAGES)
}

fn messages_text(payload: &StandardLoggingPayload) -> String {
    serde_json::to_string(payload.messages.as_ref().expect("messages are set"))
        .expect("the messages serialize")
}

#[tokio::test]
async fn callback_logger_queues_success_events() {
    let (logger, _records) = Harness::standard().build();
    let queued = payload();
    logger
        .async_log_success_event(event(queued.clone()))
        .await
        .expect("the event is recorded");
    let pending = logger.queue.pending();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].id, queued.id);
}

#[tokio::test]
async fn callback_logger_queues_failure_events() {
    let (logger, _records) = Harness::standard().build();
    let mut queued = payload();
    queued.status = LoggingStatus::Failure;
    logger
        .async_log_failure_event(event(queued.clone()))
        .await
        .expect("the event is recorded");
    let pending = logger.queue.pending();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].status, LoggingStatus::Failure);
}

#[tokio::test]
async fn callback_logger_reports_the_callback_name() {
    let (logger, _records) = Harness::standard().build();
    assert_eq!(logger.callback_name(), "aws_sqs");
}

#[tokio::test]
async fn callback_logger_strips_base64_when_enabled() {
    let mut harness = Harness::standard();
    harness.settings.strip_base64_files = true;
    let (logger, _records) = harness.build();
    let queued = payload_with_base64();
    logger
        .async_log_success_event(event(queued))
        .await
        .expect("the event is recorded");
    let messages = messages_text(&logger.queue.pending()[0]);
    assert!(messages.contains("[BASE64_REDACTED]"));
    assert!(!messages.contains("iVBORw0KGgo="));
}

#[tokio::test]
async fn callback_logger_keeps_base64_when_disabled() {
    let (logger, _records) = Harness::standard().build();
    let queued = payload_with_base64();
    logger
        .async_log_success_event(event(queued))
        .await
        .expect("the event is recorded");
    let messages = messages_text(&logger.queue.pending()[0]);
    assert!(messages.contains("iVBORw0KGgo="));
    assert!(!messages.contains("[BASE64_REDACTED]"));
}

#[tokio::test]
async fn callback_logger_uses_the_configured_base64_depth() {
    let settings = settings_from_environment(
        &[
            ("sqs_queue_url", super::support::QUEUE_URL),
            ("sqs_region_name", "us-east-1"),
            ("sqs_strip_base64_files", "true"),
        ],
        &[("DEFAULT_MAX_RECURSE_DEPTH_SENSITIVE_DATA_MASKER", "0")],
    )
    .expect("the settings resolve");
    let mut harness = Harness::standard();
    harness.settings = settings;
    let (logger, _records) = harness.build();
    logger
        .async_log_success_event(event(payload_with_base64()))
        .await
        .expect("the event is recorded");
    let messages = messages_text(&logger.queue.pending()[0]);
    assert!(messages.contains("[MAX_DEPTH_REACHED]"));
}
