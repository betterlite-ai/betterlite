#![cfg(test)]

use std::collections::BTreeMap;

use opentelemetry::Value;
use opentelemetry::trace::{Span as _, Tracer as _, TracerProvider as _};
use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider, SpanData};
use rstest::rstest;

use super::set_attributes;
use crate::otel::legacy_attributes::context::AttributeContext;
use crate::otel::legacy_attributes::error::LegacyAttributesError;
use crate::otel::legacy_policy::capture::CaptureMode;
use crate::otel::legacy_policy::config::OpenTelemetryConfig;
use crate::otel::model::metadata::wire::MetadataMap;
use crate::otel::model::utils::coercion::OtelModelValue;

const GENERIC_ONLY_KEYS: [&str; 4] = [
    "hidden_params",
    "litellm.call_id",
    "metadata.trace_name",
    "gen_ai.request.id",
];

fn map(text: &str) -> MetadataMap {
    serde_json::from_str(text).expect("map")
}

fn value(text: &str) -> OtelModelValue {
    serde_json::from_str(text).expect("value")
}

fn context<'request>(
    config: &'request OpenTelemetryConfig,
    callback_name: Option<&'request str>,
) -> AttributeContext<'request> {
    AttributeContext {
        callback_name,
        config,
        capture_mode: CaptureMode::SpanAndEvent,
    }
}

fn export(
    context: &AttributeContext<'_>,
    kwargs: &MetadataMap,
    response: Option<&OtelModelValue>,
) -> (SpanData, Result<(), LegacyAttributesError>) {
    let exporter = InMemorySpanExporter::default();
    let provider = SdkTracerProvider::builder()
        .with_simple_exporter(exporter.clone())
        .build();
    let mut span = provider.tracer("b3856-dispatch").start("case");
    let result = set_attributes(&mut span, context, kwargs, response);
    span.end();
    let spans = exporter.get_finished_spans().expect("spans");
    (spans.into_iter().next().expect("one span"), result)
}

fn run(
    context: &AttributeContext<'_>,
    kwargs: &MetadataMap,
    response: Option<&OtelModelValue>,
) -> (BTreeMap<String, Value>, Result<(), LegacyAttributesError>) {
    let (data, result) = export(context, kwargs, response);
    let attributes = data
        .attributes
        .iter()
        .map(|pair| (pair.key.to_string(), pair.value.clone()))
        .collect();
    (attributes, result)
}

fn event_names(context: &AttributeContext<'_>, kwargs: &MetadataMap) -> Vec<String> {
    let (data, result) = export(context, kwargs, None);
    result.expect("langtrace attributes");
    data.events
        .iter()
        .map(|event| event.name.to_string())
        .collect()
}

fn chat_kwargs() -> MetadataMap {
    map(r#"{
            "model": "gpt-4o",
            "messages": [{"role": "user", "content": "hello"}],
            "optional_params": {"stream": true, "max_tokens": 50},
            "litellm_params": {"custom_llm_provider": "openai"},
            "standard_logging_object": {
                "id": "chatcmpl-1",
                "call_type": "acompletion",
                "litellm_call_id": "call-1",
                "request_id": "request-1",
                "metadata": {"trace_name": "demo"},
                "hidden_params": {"litellm_model_name": "gpt-4o"}
            }
        }"#)
}

fn chat_response() -> OtelModelValue {
    value(
        r#"{"id": "chatcmpl-1", "model": "gpt-4o",
            "choices": [{"message": {"role": "assistant", "content": "hi"}, "finish_reason": "stop"}],
            "usage": {"total_tokens": 3, "completion_tokens": 1, "prompt_tokens": 2}}"#,
    )
}

fn assert_no_generic_keys(attributes: &BTreeMap<String, Value>) {
    for key in GENERIC_ONLY_KEYS {
        assert!(
            !attributes.contains_key(key),
            "{key} came from the generic flow"
        );
    }
}

#[test]
fn langtrace_hands_the_span_to_the_langtrace_writer() {
    let config = OpenTelemetryConfig::default();
    let response = chat_response();
    let (attributes, result) = run(
        &context(&config, Some("langtrace")),
        &chat_kwargs(),
        Some(&response),
    );
    result.expect("attributes");
    assert_eq!(
        attributes.get("langtrace.service.name"),
        Some(&Value::from("openai"))
    );
    assert_eq!(
        attributes.get("gen_ai.operation.name"),
        Some(&Value::from("chat"))
    );
    assert_no_generic_keys(&attributes);
}

#[test]
fn langtrace_without_a_response_writes_the_request_and_no_completion_event() {
    let config = OpenTelemetryConfig::default();
    let context = context(&config, Some("langtrace"));
    assert_eq!(
        event_names(&context, &chat_kwargs()),
        vec!["gen_ai.content.prompt".to_owned()]
    );
    let (attributes, result) = run(&context, &chat_kwargs(), None);
    result.expect("attributes");
    assert_eq!(
        attributes.get("langtrace.service.name"),
        Some(&Value::from("openai"))
    );
    assert_eq!(
        attributes.get("gen_ai.request.max_tokens"),
        Some(&Value::from(50_i64))
    );
}

#[test]
fn langfuse_otel_hands_the_span_to_the_langfuse_writer() {
    let config = OpenTelemetryConfig::default();
    let response = chat_response();
    let (attributes, result) = run(
        &context(&config, Some("langfuse_otel")),
        &chat_kwargs(),
        Some(&response),
    );
    result.expect("attributes");
    assert_eq!(
        attributes.get("langfuse.observation.type"),
        Some(&Value::from("generation"))
    );
    assert_eq!(
        attributes.get("llm.is_streaming"),
        Some(&Value::from("True"))
    );
    assert_no_generic_keys(&attributes);
}

#[test]
fn weave_otel_hands_the_span_to_the_weave_writer() {
    let config = OpenTelemetryConfig::default();
    let response = chat_response();
    let (attributes, result) = run(
        &context(&config, Some("weave_otel")),
        &chat_kwargs(),
        Some(&response),
    );
    result.expect("attributes");
    assert_eq!(
        attributes.get("openinference.span.kind"),
        Some(&Value::from("LLM"))
    );
    assert_no_generic_keys(&attributes);
}

#[rstest]
#[case::langtrace("langtrace")]
#[case::langfuse_otel("langfuse_otel")]
#[case::weave_otel("weave_otel")]
fn a_vendor_callback_exports_each_attribute_key_once(#[case] callback_name: &str) {
    let config = OpenTelemetryConfig::default();
    let (data, result) = export(
        &context(&config, Some(callback_name)),
        &chat_kwargs(),
        Some(&chat_response()),
    );
    result.expect("attributes");
    let mut names: Vec<&str> = data
        .attributes
        .iter()
        .map(|pair| pair.key.as_str())
        .collect();
    let written = names.len();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), written);
}

#[test]
fn a_repeated_vendor_key_keeps_the_last_value() {
    let config = OpenTelemetryConfig::default();
    let (data, result) = export(
        &context(&config, Some("langfuse_otel")),
        &chat_kwargs(),
        Some(&chat_response()),
    );
    result.expect("attributes");
    let input = data
        .attributes
        .iter()
        .find(|pair| pair.key.as_str() == "langfuse.observation.input")
        .map(|pair| pair.value.to_string())
        .expect("observation input");
    let parsed: OtelModelValue = serde_json::from_str(&input).expect("json text");
    assert!(parsed.as_array().is_some());
}

#[rstest]
#[case::base_otel_name(Some("otel"))]
#[case::no_name(None)]
fn other_callback_names_run_the_generic_flow(#[case] callback_name: Option<&str>) {
    let config = OpenTelemetryConfig::default();
    let response = chat_response();
    let (attributes, result) = run(
        &context(&config, callback_name),
        &chat_kwargs(),
        Some(&response),
    );
    result.expect("attributes");
    assert_eq!(
        attributes.get("metadata.trace_name"),
        Some(&Value::from("demo"))
    );
    assert_eq!(
        attributes.get("litellm.call_id"),
        Some(&Value::from("call-1"))
    );
}

#[test]
fn missing_standard_logging_object_returns_the_error_without_attributes() {
    let config = OpenTelemetryConfig::default();
    let kwargs = map(r#"{"model": "gpt-4o", "optional_params": {"stream": true}}"#);
    let response = chat_response();
    let (attributes, result) = run(&context(&config, None), &kwargs, Some(&response));
    assert!(matches!(
        result,
        Err(LegacyAttributesError::MissingStandardLoggingObject)
    ));
    assert!(attributes.is_empty());
}

#[test]
fn the_weave_writer_receives_the_redaction_answer_from_the_request() {
    let config = OpenTelemetryConfig::default();
    let passthrough = map(r#"{
            "litellm_params": {"custom_llm_provider": "anthropic"},
            "additional_args": {"complete_input_dict": {"messages": [{"role": "user", "content": "secret"}]}},
            "standard_logging_object": {"call_type": "anthropic_passthrough"}
        }"#);
    let plain = run(&context(&config, Some("weave_otel")), &passthrough, None).0;
    assert_eq!(plain.get("input.value"), Some(&Value::from("secret")));
    assert_eq!(
        plain.get("llm.input_messages.0.message.role"),
        Some(&Value::from("user"))
    );

    let redacted = map(r#"{
            "litellm_params": {"custom_llm_provider": "anthropic"},
            "additional_args": {"complete_input_dict": {"messages": [{"role": "user", "content": "secret"}]}},
            "standard_callback_dynamic_params": {"turn_off_message_logging": true},
            "standard_logging_object": {"call_type": "anthropic_passthrough"}
        }"#);
    let hidden = run(&context(&config, Some("weave_otel")), &redacted, None).0;
    assert_ne!(hidden.get("input.value"), Some(&Value::from("secret")));
    assert!(!hidden.contains_key("llm.input_messages.0.message.role"));
}
