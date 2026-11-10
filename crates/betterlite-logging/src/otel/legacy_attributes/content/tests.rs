#![cfg(test)]

use std::collections::BTreeMap;

use opentelemetry::Value;
use opentelemetry::trace::{Span as _, Tracer as _, TracerProvider as _};
use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider, Span};
use rstest::rstest;

use super::set_content_attributes;
use crate::otel::legacy_attributes::context::{AttributeContext, InferenceCall};
use crate::otel::legacy_attributes::dispatch::set_attributes;
use crate::otel::legacy_policy::capture::CaptureMode;
use crate::otel::legacy_policy::config::OpenTelemetryConfig;
use crate::otel::model::metadata::wire::MetadataMap;
use crate::otel::model::utils::coercion::OtelModelValue;
use crate::otel::vocabulary::semconv::OTELSemconvCategory;

fn map(text: &str) -> MetadataMap {
    serde_json::from_str(text).expect("map")
}

fn value(text: &str) -> OtelModelValue {
    serde_json::from_str(text).expect("value")
}

fn config(semconv: bool) -> OpenTelemetryConfig {
    let mut config = OpenTelemetryConfig::from_env(&BTreeMap::new());
    if semconv {
        config
            .semconv_stability_opt_in
            .insert(OTELSemconvCategory::GenAiLatestExperimental);
    }
    config
}

fn context(config: &OpenTelemetryConfig, capture_mode: CaptureMode) -> AttributeContext<'_> {
    AttributeContext {
        callback_name: None,
        config,
        capture_mode,
    }
}

fn record<F>(write: F) -> Vec<(String, Value)>
where
    F: FnOnce(&mut Span),
{
    let exporter = InMemorySpanExporter::default();
    let provider = SdkTracerProvider::builder()
        .with_simple_exporter(exporter.clone())
        .build();
    let mut span = provider.tracer("b3856-content").start("case");
    write(&mut span);
    span.end();
    let spans = exporter.get_finished_spans().expect("spans");
    spans
        .first()
        .expect("one span")
        .attributes
        .iter()
        .map(|pair| (pair.key.to_string(), pair.value.clone()))
        .collect()
}

fn content_attributes(config: &OpenTelemetryConfig, kwargs: &MetadataMap) -> Vec<(String, Value)> {
    let empty = MetadataMap::new();
    let payload = kwargs
        .get("standard_logging_object")
        .and_then(OtelModelValue::as_object)
        .unwrap_or(&empty);
    let call = InferenceCall {
        kwargs,
        optional_params: kwargs
            .get("optional_params")
            .and_then(OtelModelValue::as_object)
            .unwrap_or(&empty),
        litellm_params: kwargs
            .get("litellm_params")
            .and_then(OtelModelValue::as_object)
            .unwrap_or(&empty),
        payload,
        response: None,
    };
    let context = context(config, CaptureMode::SpanAndEvent);
    record(|span| set_content_attributes(span, &context, &call))
}

fn attribute<'attributes>(
    attributes: &'attributes [(String, Value)],
    key: &str,
) -> Option<&'attributes Value> {
    attributes
        .iter()
        .find(|(name, _)| name == key)
        .map(|(_, value)| value)
}

fn keys(attributes: &[(String, Value)]) -> Vec<&str> {
    attributes.iter().map(|(key, _)| key.as_str()).collect()
}

#[rstest]
#[case::no_content(CaptureMode::NoContent)]
#[case::event_only(CaptureMode::EventOnly)]
fn a_capture_mode_outside_the_span_writes_usage_but_no_content(#[case] mode: CaptureMode) {
    let kwargs = map(r#"{
            "call_type": "acompletion",
            "messages": [{"role": "user", "content": "hello"}],
            "optional_params": {"tools": [{"type": "function", "function": {"name": "lookup"}}]},
            "standard_logging_object": {"call_type": "acompletion", "request_id": "request-1"}
        }"#);
    let response = value(
        r#"{"choices": [{"message": {"role": "assistant", "content": "hi"}}], "usage": {"total_tokens": 3}}"#,
    );
    let settings = config(false);
    let context = context(&settings, mode);
    let attributes = record(|span| {
        set_attributes(span, &context, &kwargs, Some(&response)).expect("attributes");
    });
    assert_eq!(
        attribute(&attributes, "gen_ai.usage.total_tokens"),
        Some(&Value::I64(3))
    );
    assert!(!keys(&attributes).contains(&"gen_ai.input.messages"));
    assert!(!keys(&attributes).contains(&"gen_ai.output.messages"));
    assert!(!keys(&attributes).contains(&"gen_ai.operation.name"));
    assert!(!keys(&attributes).contains(&"gen_ai.request.id"));
    assert!(!keys(&attributes).contains(&"llm.request.functions.0.name"));
}

#[rstest]
#[case::span_only(CaptureMode::SpanOnly)]
#[case::span_and_event(CaptureMode::SpanAndEvent)]
fn a_capture_mode_in_the_span_writes_the_input_messages(#[case] mode: CaptureMode) {
    let kwargs = map(r#"{
            "call_type": "acompletion",
            "messages": [{"role": "user", "content": "hello"}],
            "standard_logging_object": {"call_type": "acompletion", "request_id": "request-1"}
        }"#);
    let settings = config(false);
    let context = context(&settings, mode);
    let attributes = record(|span| {
        set_attributes(span, &context, &kwargs, None).expect("attributes");
    });
    assert!(keys(&attributes).contains(&"gen_ai.input.messages"));
}

#[test]
fn tools_are_written_from_optional_params() {
    let kwargs = map(r#"{
            "optional_params": {"tools": [{"type": "function", "function": {"name": "lookup", "description": "Look up"}}]},
            "standard_logging_object": {"call_type": "acompletion"}
        }"#);
    let attributes = content_attributes(&config(false), &kwargs);
    assert_eq!(
        attribute(&attributes, "llm.request.functions.0.name"),
        Some(&Value::from("lookup"))
    );
    assert_eq!(
        attribute(&attributes, "llm.request.functions.0.description"),
        Some(&Value::from("Look up"))
    );
}

#[test]
fn input_messages_are_the_transformed_message_list() {
    let kwargs = map(r#"{
            "messages": [{"role": "user", "content": "hello"}],
            "standard_logging_object": {"call_type": "acompletion"}
        }"#);
    let attributes = content_attributes(&config(false), &kwargs);
    assert_eq!(
        attribute(&attributes, "gen_ai.input.messages"),
        Some(&Value::from(
            r#"[{"parts":[{"content":"hello","type":"text"}],"role":"user"}]"#
        ))
    );
}

#[rstest]
#[case::empty_list_wins(r#""system_instructions": [], "instructions": "be brief""#, None)]
#[case::empty_string_wins(
    r#""system_instructions": "", "instructions": "be brief", "system": "be terse""#,
    None
)]
#[case::null_falls_through(
    r#""system_instructions": null, "instructions": "be brief""#,
    Some("be brief")
)]
#[case::instructions_first(
    r#""instructions": "be brief", "system": "be terse""#,
    Some("be brief")
)]
#[case::anthropic_system(r#""system": "be terse""#, Some("be terse"))]
fn system_instructions_pick_the_first_non_null_kwarg(
    #[case] members: &str,
    #[case] expected: Option<&str>,
) {
    let kwargs = map(&format!(
        r#"{{{members}, "standard_logging_object": {{"call_type": "acompletion"}}}}"#
    ));
    let attributes = content_attributes(&config(false), &kwargs);
    assert_eq!(
        attribute(&attributes, "gen_ai.system_instructions"),
        expected.map(|text| Value::from(text.to_owned())).as_ref()
    );
}

#[test]
fn a_message_list_system_prompt_is_transformed_first() {
    let kwargs = map(r#"{
            "system_instructions": [{"role": "system", "content": "be brief"}],
            "standard_logging_object": {"call_type": "acompletion"}
        }"#);
    let attributes = content_attributes(&config(false), &kwargs);
    assert_eq!(
        attribute(&attributes, "gen_ai.system_instructions"),
        Some(&Value::from(
            r#"[{"parts":[{"content":"be brief","type":"text"}],"role":"system"}]"#
        ))
    );
}

#[rstest]
#[case::completion("completion", "chat")]
#[case::empty_call_type("", "chat")]
#[case::acompletion("acompletion", "acompletion")]
#[case::embedding("embedding", "embedding")]
fn the_operation_name_comes_from_the_payload_call_type(
    #[case] call_type: &str,
    #[case] expected: &str,
) {
    let kwargs = map(&format!(
        r#"{{"standard_logging_object": {{"call_type": "{call_type}"}}}}"#
    ));
    let attributes = content_attributes(&config(false), &kwargs);
    assert_eq!(
        attribute(&attributes, "gen_ai.operation.name"),
        Some(&Value::from(expected.to_owned()))
    );
}

#[test]
fn an_absent_payload_call_type_is_chat() {
    let kwargs = map(r#"{"standard_logging_object": {}}"#);
    let attributes = content_attributes(&config(false), &kwargs);
    assert_eq!(
        attribute(&attributes, "gen_ai.operation.name"),
        Some(&Value::from("chat"))
    );
}

#[test]
fn semconv_mode_names_the_operation_after_the_kwargs_call_type() {
    let kwargs = map(
        r#"{"call_type": "aembedding", "standard_logging_object": {"call_type": "completion"}}"#,
    );
    let attributes = content_attributes(&config(true), &kwargs);
    assert_eq!(
        attribute(&attributes, "gen_ai.operation.name"),
        Some(&Value::from("embeddings"))
    );
}

#[test]
fn the_request_id_is_written_when_truthy() {
    let kwargs = map(
        r#"{"standard_logging_object": {"call_type": "acompletion", "request_id": "request-1"}}"#,
    );
    let attributes = content_attributes(&config(false), &kwargs);
    assert_eq!(
        attribute(&attributes, "gen_ai.request.id"),
        Some(&Value::from("request-1"))
    );
    let empty =
        map(r#"{"standard_logging_object": {"call_type": "acompletion", "request_id": ""}}"#);
    let without_request_id = content_attributes(&config(false), &empty);
    assert!(!keys(&without_request_id).contains(&"gen_ai.request.id"));
}
