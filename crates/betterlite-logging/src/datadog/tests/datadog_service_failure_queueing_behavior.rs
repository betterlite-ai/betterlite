use betterlite_models::decisions::Json;

use crate::datadog::payloads::status::DataDogStatus;
use crate::datadog::service_events::async_service_failure_hook;
use crate::logging_service_events::types::ServiceType;

use super::support::{
    Intake, build_logger, field, json, last_queued, message, service_payload, text,
};

#[test]
fn datadog_service_failure_queueing_behavior() {
    let environment = [
        ("DD_BATCH_SIZE", "1"),
        ("DD_ENV", "prod"),
        ("HOSTNAME", "host-1"),
    ];
    let (logger, transport) = build_logger(&environment, Intake::Accepted);
    let payload = service_payload(ServiceType::Redis, true, Some("connection refused"), 0.25);
    async_service_failure_hook(&logger, &payload, None);

    // the queue already holds the batch size, and a service failure never flushes it
    assert_eq!(logger.queue.pending().len(), 1);
    assert_eq!(transport.post_count(), 0);
    let queued = last_queued(&logger);
    assert_eq!(queued.status, DataDogStatus::Warn);
    assert_eq!(
        queued.ddtags,
        "env:prod,service:litellm-server,version:unknown,HOSTNAME:host-1,POD_NAME:unknown"
    );
    assert_eq!(queued.trace_id, None);
    assert_eq!(queued.span_id, None);
}

#[test]
fn datadog_service_failure_message_behavior() {
    let (logger, _) = build_logger(&[], Intake::Accepted);
    let payload = service_payload(ServiceType::Redis, true, Some("connection refused"), 0.25);
    async_service_failure_hook(&logger, &payload, None);

    // the queued message carries the fields the service payload dumps
    let body = message(&last_queued(&logger));
    assert_eq!(field(&body, "is_error"), &Json::Bool(true));
    assert_eq!(text(field(&body, "error")), "connection refused");
    assert_eq!(text(field(&body, "service")), "redis");
    assert_eq!(text(field(&body, "call_type")), "acompletion");
    assert_eq!(field(&body, "duration"), &json("0.25"));
    assert_eq!(field(&body, "caller"), &Json::Null);
    assert_eq!(field(&body, "event_metadata"), &Json::Null);
}

#[test]
fn datadog_service_failure_without_error_message_behavior() {
    let (logger, transport) = build_logger(&[("DD_BATCH_SIZE", "1")], Intake::Accepted);
    let postgres = service_payload(ServiceType::Postgres, true, None, 1.5);
    async_service_failure_hook(&logger, &postgres, None);
    let body = message(&last_queued(&logger));
    assert_eq!(text(field(&body, "service")), "postgres");
    assert_eq!(field(&body, "error"), &Json::Null);
    assert_eq!(field(&body, "duration"), &json("1.5"));

    // a second failure still only queues, even at a batch size of one
    async_service_failure_hook(&logger, &postgres, None);
    assert_eq!(logger.queue.pending().len(), 2);
    assert_eq!(transport.post_count(), 0);
}
