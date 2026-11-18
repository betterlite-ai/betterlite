#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, UNIX_EPOCH};

use opentelemetry::trace::{SpanKind, Status, TracerProvider as _};
use opentelemetry::{Array, StringValue, Value};
use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider, SpanData};

use super::{APP_NAME, CALLBACK_NAME, DISABLE_BATCH, SPAN_NAME, TraceloopConfig, TraceloopLogger};
use crate::callback_logger::CallbackLogger as _;
use crate::otel::callbacks::tests::{
    DEFAULT_MESSAGES, SUCCESS_RESPONSE, event, from_payload, payload, payload_with,
};

fn logger() -> (TraceloopLogger, InMemorySpanExporter) {
    let exporter = InMemorySpanExporter::default();
    let provider = SdkTracerProvider::builder()
        .with_simple_exporter(exporter.clone())
        .build();
    let tracer = provider.tracer("traceloop-tests");
    (TraceloopLogger { tracer }, exporter)
}

fn only_span(exporter: &InMemorySpanExporter) -> SpanData {
    let spans = exporter.get_finished_spans().expect("the finished spans");
    assert_eq!(spans.len(), 1, "one span is exported");
    spans.into_iter().next().expect("the single span")
}

fn attributes(span: &SpanData) -> BTreeMap<String, Value> {
    span.attributes
        .iter()
        .map(|pair| (pair.key.to_string(), pair.value.clone()))
        .collect()
}

fn strings(items: &[&str]) -> Value {
    Value::Array(Array::String(
        items
            .iter()
            .map(|item| StringValue::from((*item).to_owned()))
            .collect(),
    ))
}

#[test]
fn traceloop_emits_one_client_span_named_litellm_completion() {
    let (logger, exporter) = logger();
    logger
        .log_success_event(&event())
        .expect("the event is logged");
    let span = only_span(&exporter);
    assert_eq!(span.name, SPAN_NAME);
    assert_eq!(span.span_kind, SpanKind::Client);
    assert!(matches!(span.status, Status::Unset));
    // LiteLLM hands whole seconds to an argument the SDK reads as nanoseconds
    assert_eq!(
        span.start_time,
        UNIX_EPOCH + Duration::from_nanos(1_704_067_200)
    );
    assert_eq!(
        span.end_time,
        UNIX_EPOCH + Duration::from_nanos(1_704_067_201)
    );
}

#[test]
fn traceloop_sets_the_request_attributes() {
    let (logger, exporter) = logger();
    logger
        .log_success_event(&event())
        .expect("the event is logged");
    let attributes = attributes(&only_span(&exporter));
    assert_eq!(
        attributes.get("gen_ai.request.model"),
        Some(&Value::from("gpt-4o"))
    );
    assert_eq!(
        attributes.get("llm.chat.stop_sequences"),
        Some(&strings(&["END"]))
    );
    assert_eq!(
        attributes.get("llm.frequency_penalty"),
        Some(&Value::F64(0.1))
    );
    assert_eq!(
        attributes.get("llm.presence_penalty"),
        Some(&Value::F64(0.3))
    );
    assert_eq!(
        attributes.get("gen_ai.request.top_p"),
        Some(&Value::F64(0.9))
    );
    assert_eq!(
        attributes.get("gen_ai.request.max_tokens"),
        Some(&Value::I64(16))
    );
    assert_eq!(
        attributes.get("gen_ai.request.temperature"),
        Some(&Value::F64(0.2))
    );
    assert_eq!(attributes.get("llm.user"), Some(&Value::from("end-user-1")));
    assert_eq!(
        attributes.get("llm.request.functions"),
        Some(&strings(&["lookup"]))
    );
}

#[test]
fn traceloop_sets_the_prompt_completion_and_usage_attributes() {
    let (logger, exporter) = logger();
    logger
        .log_success_event(&event())
        .expect("the event is logged");
    let attributes = attributes(&only_span(&exporter));
    assert_eq!(
        attributes.get("gen_ai.prompt.0.role"),
        Some(&Value::from("user"))
    );
    assert_eq!(
        attributes.get("gen_ai.prompt.0.content"),
        Some(&Value::from("Hello"))
    );
    assert_eq!(
        attributes.get("gen_ai.response.model"),
        Some(&Value::from("gpt-4o-2024-08-06"))
    );
    assert_eq!(
        attributes.get("llm.usage.total_tokens"),
        Some(&Value::I64(11))
    );
    assert_eq!(
        attributes.get("gen_ai.usage.completion_tokens"),
        Some(&Value::I64(3))
    );
    assert_eq!(
        attributes.get("gen_ai.usage.prompt_tokens"),
        Some(&Value::I64(8))
    );
    assert_eq!(
        attributes.get("gen_ai.completion.0.finish_reason"),
        Some(&Value::from("stop"))
    );
    assert_eq!(
        attributes.get("gen_ai.completion.0.role"),
        Some(&Value::from("assistant"))
    );
    assert_eq!(
        attributes.get("gen_ai.completion.0.content"),
        Some(&Value::from("Hi there"))
    );
}

#[test]
fn traceloop_drops_a_tool_list_python_cannot_encode() {
    let (logger, exporter) = logger();
    let event = from_payload(&payload_with(
        SUCCESS_RESPONSE,
        DEFAULT_MESSAGES,
        r#"{"tools": [{"type": "function", "function": {"name": "lookup"}}], "user": "end-user-1"}"#,
    ));
    logger
        .log_success_event(&event)
        .expect("the event is logged");
    let attributes = attributes(&only_span(&exporter));
    assert!(
        !attributes.contains_key("llm.request.functions"),
        "a sequence of objects is not an attribute value"
    );
    assert_eq!(attributes.get("llm.user"), Some(&Value::from("end-user-1")));
}

#[test]
fn traceloop_marks_a_failure_with_an_error_status_and_an_exception_event() {
    let (logger, exporter) = logger();
    let payload = payload(SUCCESS_RESPONSE, DEFAULT_MESSAGES).replace(
        r#""hidden_params": {}"#,
        r#""hidden_params": {}, "error_information": {"error_message": "boom"}"#,
    );
    logger
        .log_failure_event(&from_payload(&payload))
        .expect("the event is logged");
    let span = only_span(&exporter);
    let Status::Error { description } = span.status else {
        panic!("the span carries an error status");
    };
    assert_eq!(description, "boom");
    let event = span
        .events
        .iter()
        .find(|event| event.name.as_ref() == "exception")
        .expect("the exception event");
    assert!(event.attributes.iter().any(|attribute| {
        attribute.key.as_str() == "exception.message" && attribute.value == Value::from("boom")
    }));
}

#[test]
fn traceloop_stops_before_the_completions_when_choices_are_missing() {
    let (logger, exporter) = logger();
    let event = from_payload(&payload(
        r#"{"id": "chatcmpl-response-1", "model": "gpt-4o-2024-08-06"}"#,
        DEFAULT_MESSAGES,
    ));
    logger
        .log_success_event(&event)
        .expect("the event is logged");
    let span = only_span(&exporter);
    let attributes = attributes(&span);
    assert!(matches!(span.status, Status::Unset));
    assert_eq!(
        attributes.get("gen_ai.response.model"),
        Some(&Value::from("gpt-4o-2024-08-06"))
    );
    assert!(!attributes.contains_key("gen_ai.completion.0.role"));
}

#[test]
fn traceloop_stops_before_the_completions_when_choices_are_null() {
    let (logger, exporter) = logger();
    let event = from_payload(&payload(
        r#"{"id": "chatcmpl-response-1", "choices": null}"#,
        DEFAULT_MESSAGES,
    ));
    logger
        .log_success_event(&event)
        .expect("the event is logged");
    let span = only_span(&exporter);
    let attributes = attributes(&span);
    assert!(matches!(span.status, Status::Unset));
    assert!(!attributes.contains_key("gen_ai.completion.0.role"));
}

#[test]
fn traceloop_stops_before_the_prompts_when_messages_are_missing() {
    let (logger, exporter) = logger();
    logger
        .log_success_event(&from_payload(&payload(SUCCESS_RESPONSE, "null")))
        .expect("the event is logged");
    let attributes = attributes(&only_span(&exporter));
    assert!(!attributes.contains_key("gen_ai.prompt.0.role"));
    assert!(!attributes.contains_key("gen_ai.response.model"));
}

#[test]
fn traceloop_stops_before_the_response_when_the_response_is_missing() {
    let (logger, exporter) = logger();
    logger
        .log_success_event(&from_payload(&payload("null", DEFAULT_MESSAGES)))
        .expect("the event is logged");
    let attributes = attributes(&only_span(&exporter));
    assert_eq!(
        attributes.get("gen_ai.prompt.0.content"),
        Some(&Value::from("Hello"))
    );
    assert!(!attributes.contains_key("gen_ai.response.model"));
}

#[tokio::test]
async fn traceloop_async_logs_a_success_on_the_litellm_completion_span() {
    let (logger, exporter) = logger();
    logger
        .async_log_success_event(Arc::new(event()))
        .await
        .expect("the event is logged");
    let span = only_span(&exporter);
    assert_eq!(span.name, SPAN_NAME);
    assert_eq!(
        attributes(&span).get("gen_ai.prompt.0.role"),
        Some(&Value::from("user"))
    );
}

#[tokio::test]
async fn traceloop_async_logs_a_failure_with_the_error_status() {
    let (logger, exporter) = logger();
    let payload = payload(SUCCESS_RESPONSE, DEFAULT_MESSAGES).replace(
        r#""hidden_params": {}"#,
        r#""hidden_params": {}, "error_information": {"error_message": "boom"}"#,
    );
    logger
        .async_log_failure_event(Arc::new(from_payload(&payload)))
        .await
        .expect("the event is logged");
    let span = only_span(&exporter);
    assert!(matches!(span.status, Status::Error { .. }));
}

#[test]
fn traceloop_defaults_the_sdk_configuration_and_callback_name() {
    let config = TraceloopConfig::default();
    assert_eq!(config.app_name, APP_NAME);
    assert_eq!(config.app_name, "Litellm-Server");
    assert_eq!(config.disable_batch, DISABLE_BATCH);
    assert!(config.disable_batch);
    let (logger, _exporter) = logger();
    assert_eq!(logger.callback_name(), CALLBACK_NAME);
    assert_eq!(CALLBACK_NAME, "traceloop");
}
