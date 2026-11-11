use std::sync::Arc;

use chrono::DateTime;

use super::support::{attribute, default_fixture, event, json, payload};
use crate::callback_logger::CallbackLogger;
use crate::logging_pre_post::hooks::{ApiCallLogger, PostCallEvent, PreCallEvent};
use crate::newrelic_agent::events::{ERROR_METRIC_NAME, MESSAGE_EVENT_NAME, SUMMARY_EVENT_NAME};
use crate::newrelic_agent::logger::NewRelicLogger;

#[test]
fn callback_logger_reports_its_name_and_message_logging_flags() {
    let fixture = default_fixture();
    let logger = fixture.build();
    let callback: &dyn CallbackLogger = logger.as_ref();
    assert_eq!(callback.callback_name(), "newrelic");
    assert!(!callback.turn_off_message_logging());
    assert!(callback.message_logging());
    assert!(!callback.redacts_messages_itself());
    let concrete: Arc<NewRelicLogger> = Arc::clone(&logger);
    let boxed: Arc<dyn CallbackLogger> = concrete;
    assert_eq!(boxed.callback_name(), "newrelic");

    let mut quiet = default_fixture();
    quiet.turn_off(true);
    let quiet_logger = quiet.build();
    let quiet_callback: &dyn CallbackLogger = quiet_logger.as_ref();
    assert!(quiet_callback.turn_off_message_logging());
}

#[tokio::test]
async fn callback_logger_uses_the_payload_trace_id_on_both_event_types() {
    let fixture = default_fixture();
    let logger = fixture.build();
    let mut standard = payload();
    standard.trace_id = "4bf92f3577b34da6a3ce929d0e0e4736".to_owned();
    logger
        .async_log_success_event(event(standard))
        .await
        .unwrap();
    let trace_id = json(r#""4bf92f3577b34da6a3ce929d0e0e4736""#);
    let summary = fixture.application.events_named(SUMMARY_EVENT_NAME);
    let messages = fixture.application.events_named(MESSAGE_EVENT_NAME);
    assert_eq!(attribute(&summary[0], "trace_id"), Some(&trace_id));
    assert_eq!(attribute(&messages[0], "trace_id"), Some(&trace_id));
}

#[tokio::test]
async fn callback_logger_success_events_record_the_ai_events() {
    let fixture = default_fixture();
    let logger = fixture.build();
    let concrete: Arc<NewRelicLogger> = Arc::clone(&logger);
    let callback: Arc<dyn CallbackLogger> = concrete;
    callback
        .async_log_success_event(event(payload()))
        .await
        .unwrap();
    assert_eq!(
        fixture.application.events_named(SUMMARY_EVENT_NAME).len(),
        1
    );
    assert_eq!(
        fixture.application.events_named(MESSAGE_EVENT_NAME).len(),
        2
    );
}

#[tokio::test]
async fn callback_logger_failure_events_record_the_error_metric_only() {
    let fixture = default_fixture();
    let logger = fixture.build();
    let concrete: Arc<NewRelicLogger> = Arc::clone(&logger);
    let callback: Arc<dyn CallbackLogger> = concrete;
    callback
        .async_log_failure_event(event(payload()))
        .await
        .unwrap();
    assert_eq!(
        fixture.application.metrics_named(ERROR_METRIC_NAME),
        vec![1]
    );
    assert!(fixture.application.events().is_empty());
}

#[tokio::test]
async fn callback_logger_sync_and_async_success_events_share_one_path() {
    let fixture = default_fixture();
    let logger = fixture.build();
    logger.log_success_event(&event(payload())).unwrap();
    logger
        .async_log_success_event(event(payload()))
        .await
        .unwrap();
    assert_eq!(
        fixture.application.events_named(SUMMARY_EVENT_NAME).len(),
        2
    );
    assert_eq!(
        fixture.application.events_named(MESSAGE_EVENT_NAME).len(),
        4
    );
}

#[test]
fn callback_logger_pre_and_post_api_calls_are_noops() {
    let fixture = default_fixture();
    let logger = fixture.build();
    let api: &dyn ApiCallLogger = logger.as_ref();
    let details = json(r#"{"model": "gpt-4o"}"#);
    let messages = json("[]");
    let pre = PreCallEvent {
        model: "gpt-4o",
        messages: &messages,
        details: &details,
    };
    api.log_pre_api_call(&pre).unwrap();
    let start_time = DateTime::parse_from_rfc3339("2024-01-01T00:00:00+00:00").unwrap();
    let post = PostCallEvent {
        details: &details,
        start_time,
    };
    api.log_post_api_call(&post).unwrap();
    assert!(fixture.application.events().is_empty());
    assert!(fixture.application.metrics().is_empty());
}
