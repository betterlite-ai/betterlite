use std::sync::Arc;

use betterlite_logging::callback_logger::CallbackLogger as _;
use betterlite_logging::callback_logger::event::LoggerEvent;
use betterlite_logging::fields::payload::StandardLoggingPayload;
use betterlite_logging::otel::legacy_lifecycle::logger::LegacyOtelLogger;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use opentelemetry::trace::Status;
use opentelemetry_sdk::trace::SpanData;

use crate::support::Rig;

const PAYLOAD: &str = r#"{
    "id": "chatcmpl-1",
    "trace_id": "trace-1",
    "session_id": "session-1",
    "call_type": "completion",
    "response_cost": 0.5,
    "status": "success",
    "total_tokens": 5,
    "prompt_tokens": 2,
    "completion_tokens": 3,
    "startTime": 1704067200.0,
    "endTime": 1704067201.5,
    "completionStartTime": 1704067200.5,
    "response_time": 1.0,
    "model": "gpt-4o",
    "messages": [{"role": "user", "content": "secret request"}],
    "api_base": "https://api.openai.com",
    "metadata": {},
    "saved_cache_cost": 0.0,
    "request_tags": [],
    "model_parameters": {},
    "hidden_params": {}
}"#;

fn event(id: &str) -> LoggerEvent {
    let mut payload: StandardLoggingPayload = serde_json::from_str(PAYLOAD).expect("payload");
    id.clone_into(&mut payload.id);
    LoggerEvent {
        payload: Arc::new(payload),
        dynamic_params: Arc::new(Json::Object(IndexMap::new())),
        litellm_metadata: Arc::new(IndexMap::new()),
        stream: false,
        guardrail_snapshot: None,
    }
}

fn has_secret(span: &SpanData) -> bool {
    span.attributes
        .iter()
        .any(|attribute| format!("{:?}", attribute.value).contains("secret request"))
}

#[test]
fn callback_logger_exposes_its_name_and_logging_flags() {
    let rig = Rig::new();
    let logger = LegacyOtelLogger::new(Arc::new(rig.in_memory(None)));
    assert_eq!(logger.callback_name(), "opentelemetry");
    assert!(logger.message_logging());
    assert!(!logger.turn_off_message_logging());
    assert!(!logger.redacts_messages_itself());

    let named = LegacyOtelLogger::new(Arc::new(rig.in_memory(Some("arize"))));
    assert_eq!(named.callback_name(), "arize");
}

#[test]
fn callback_logger_success_event_writes_primary_and_raw_spans() {
    let rig = Rig::new();
    let logger = LegacyOtelLogger::new(Arc::new(rig.in_memory(None)));
    logger.log_success_event(&event("call-1")).expect("success");
    assert_eq!(rig.names(), vec!["litellm_request", "raw_gen_ai_request"]);
    assert_eq!(rig.span("litellm_request").status, Status::Ok);
}

#[test]
fn callback_logger_failure_event_writes_error_span() {
    let rig = Rig::new();
    let logger = LegacyOtelLogger::new(Arc::new(rig.in_memory(None)));
    logger.log_failure_event(&event("call-1")).expect("failure");
    let spans = rig.spans();
    assert_eq!(spans.len(), 1);
    assert_eq!(spans[0].name, "litellm_request");
    assert!(matches!(spans[0].status, Status::Error { .. }));
}

#[tokio::test]
async fn callback_logger_async_events_use_the_lifecycle_handlers() {
    let rig = Rig::new();
    let logger = LegacyOtelLogger::new(Arc::new(rig.in_memory(None)));
    logger
        .async_log_success_event(Arc::new(event("call-1")))
        .await
        .expect("async success");
    logger
        .async_log_failure_event(Arc::new(event("call-2")))
        .await
        .expect("async failure");
    assert_eq!(rig.names().len(), 3);
}

#[test]
fn callback_logger_sync_success_dedupes_repeat() {
    let rig = Rig::new();
    let logger = LegacyOtelLogger::new(Arc::new(rig.in_memory(None)));
    let request = event("call-1");
    logger.log_success_event(&request).expect("first");
    logger.log_success_event(&request).expect("repeat");
    assert_eq!(rig.names().len(), 2);
}

#[test]
fn callback_logger_sync_failure_dedupes_repeat() {
    let rig = Rig::new();
    let logger = LegacyOtelLogger::new(Arc::new(rig.in_memory(None)));
    let request = event("call-1");
    logger.log_failure_event(&request).expect("first");
    logger.log_failure_event(&request).expect("repeat");
    assert_eq!(rig.names(), vec!["litellm_request"]);
}

fn success_redacts(value: Json) {
    let rig = Rig::new();
    let logger = LegacyOtelLogger::new(Arc::new(rig.in_memory(None)));
    let mut request = event("call-redacted");
    request.dynamic_params = Arc::new(Json::Object(IndexMap::from([(
        "turn_off_message_logging".to_owned(),
        value,
    )])));
    logger.log_success_event(&request).expect("success");
    assert!(!has_secret(&rig.span("litellm_request")));
}

#[test]
fn callback_logger_bool_turn_off_redacts_span_content() {
    let rig = Rig::new();
    let logger = LegacyOtelLogger::new(Arc::new(rig.in_memory(None)));
    logger
        .log_success_event(&event("call-visible"))
        .expect("success");
    assert!(has_secret(&rig.span("litellm_request")));
    success_redacts(Json::Bool(true));
}
