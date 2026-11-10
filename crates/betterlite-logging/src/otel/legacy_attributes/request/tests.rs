#![cfg(test)]

use std::collections::BTreeMap;

use opentelemetry::Value;
use opentelemetry::trace::{Span as _, Tracer as _, TracerProvider as _};
use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider, Span};
use rstest::rstest;

use super::set_request_attributes;
use crate::otel::legacy_attributes::context::{AttributeContext, InferenceCall};
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

fn request_attributes(
    config: &OpenTelemetryConfig,
    kwargs: &MetadataMap,
    response: Option<&OtelModelValue>,
) -> Vec<(String, Value)> {
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
        response,
    };
    let context = AttributeContext {
        callback_name: None,
        config,
        capture_mode: CaptureMode::SpanAndEvent,
    };
    let exporter = InMemorySpanExporter::default();
    let provider = SdkTracerProvider::builder()
        .with_simple_exporter(exporter.clone())
        .build();
    let mut span: Span = provider.tracer("b3856-request").start("case");
    set_request_attributes(&mut span, &context, &call);
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

fn full_kwargs() -> MetadataMap {
    map(r#"{
            "model": "gpt-4o",
            "optional_params": {"max_tokens": 100, "temperature": 0.5, "top_p": 0.9, "stream": true, "user": "u1"},
            "litellm_params": {"custom_llm_provider": "openai"},
            "standard_logging_object": {
                "metadata": {"trace_name": "demo", "user_api_key_request_route": "/v1/chat/completions"},
                "hidden_params": {"litellm_model_name": "gpt-4o-2024"},
                "cost_breakdown": {"input_cost": 0.001, "output_cost": null},
                "call_type": "acompletion",
                "litellm_call_id": "call-1",
                "id": "chatcmpl-1",
                "model_group": "public-model",
                "model": "gpt-4o",
                "model_parameters": {"service_tier": "flex"}
            }
        }"#)
}

fn full_response() -> OtelModelValue {
    value(
        r#"{"id": "chatcmpl-1", "model": "gpt-4o",
            "usage": {"total_tokens": 30, "completion_tokens": 10, "prompt_tokens": 20}}"#,
    )
}

#[test]
fn request_attributes_follow_litellm_order() {
    let response = full_response();
    let attributes = request_attributes(&config(false), &full_kwargs(), Some(&response));
    assert_eq!(
        keys(&attributes),
        vec![
            "metadata.trace_name",
            "metadata.user_api_key_request_route",
            "hidden_params",
            "http.route",
            "litellm.model_group",
            "litellm.provider.model",
            "gen_ai.cost.input_cost",
            "gen_ai.request.model",
            "llm.request.type",
            "gen_ai.system",
            "gen_ai.request.max_tokens",
            "gen_ai.request.temperature",
            "gen_ai.request.top_p",
            "llm.is_streaming",
            "llm.user",
            "gen_ai.response.id",
            "litellm.call_id",
            "gen_ai.response.model",
            "gen_ai.openai.request.service_tier",
            "gen_ai.usage.total_tokens",
            "gen_ai.usage.output_tokens",
            "gen_ai.usage.input_tokens",
        ]
    );
}

#[test]
fn request_attribute_values_come_from_the_payload_and_the_request() {
    let response = full_response();
    let attributes = request_attributes(&config(false), &full_kwargs(), Some(&response));
    assert_eq!(
        attribute(&attributes, "hidden_params"),
        Some(&Value::from(r#"{"litellm_model_name":"gpt-4o-2024"}"#))
    );
    assert_eq!(
        attribute(&attributes, "llm.request.type"),
        Some(&Value::from("acompletion"))
    );
    assert_eq!(
        attribute(&attributes, "gen_ai.request.model"),
        Some(&Value::from("gpt-4o"))
    );
    assert_eq!(
        attribute(&attributes, "litellm.provider.model"),
        Some(&Value::from("gpt-4o-2024"))
    );
}

#[test]
fn hidden_params_and_cost_breakdown_skip_falsy_and_null_entries() {
    let kwargs = map(r#"{"standard_logging_object": {
            "call_type": "acompletion",
            "hidden_params": {},
            "cost_breakdown": {"input_cost": 0.001, "output_cost": null}
        }}"#);
    let attributes = request_attributes(&config(false), &kwargs, None);
    assert_eq!(
        keys(&attributes),
        vec![
            "gen_ai.cost.input_cost",
            "llm.request.type",
            "gen_ai.system",
            "llm.is_streaming",
        ]
    );

    let empty_cost =
        map(r#"{"standard_logging_object": {"call_type": "acompletion", "cost_breakdown": {}}}"#);
    let cost_attributes = request_attributes(&config(false), &empty_cost, None);
    assert!(
        !keys(&cost_attributes)
            .iter()
            .any(|key| key.starts_with("gen_ai.cost"))
    );
}

#[test]
fn the_provider_defaults_to_unknown_and_a_null_provider_reads_as_empty() {
    let absent = map(r#"{"standard_logging_object": {"call_type": "acompletion"}}"#);
    let attributes = request_attributes(&config(false), &absent, None);
    assert_eq!(
        attribute(&attributes, "gen_ai.system"),
        Some(&Value::from("Unknown"))
    );

    let null_provider = map(r#"{"litellm_params": {"custom_llm_provider": null},
            "standard_logging_object": {"call_type": "acompletion"}}"#);
    let null_attributes = request_attributes(&config(false), &null_provider, None);
    assert_eq!(
        attribute(&null_attributes, "gen_ai.system"),
        Some(&Value::from(""))
    );
}

#[test]
fn semconv_mode_writes_the_provider_name_and_the_semconv_request_attributes() {
    let kwargs = map(r#"{
            "optional_params": {"stream": true, "max_tokens": 5, "frequency_penalty": 0},
            "litellm_params": {"custom_llm_provider": "anthropic"},
            "standard_logging_object": {
                "call_type": "acompletion",
                "metadata": {"usage_object": {"cache_read_input_tokens": 7}}
            }
        }"#);
    let attributes = request_attributes(&config(true), &kwargs, None);
    assert_eq!(
        attribute(&attributes, "gen_ai.provider.name"),
        Some(&Value::from("anthropic"))
    );
    assert!(!keys(&attributes).contains(&"gen_ai.system"));
    assert!(!keys(&attributes).contains(&"llm.is_streaming"));
    assert_eq!(
        attribute(&attributes, "gen_ai.request.stream"),
        Some(&Value::Bool(true))
    );
    assert_eq!(
        attribute(&attributes, "gen_ai.request.frequency_penalty"),
        Some(&Value::I64(0))
    );
    assert_eq!(
        attribute(&attributes, "gen_ai.usage.cache_read.input_tokens"),
        Some(&Value::I64(7))
    );
}

#[rstest]
#[case::truthy_stream(r#"{"stream": true}"#, "True")]
#[case::false_stream(r#"{"stream": false}"#, "False")]
#[case::null_stream(r#"{"stream": null}"#, "None")]
#[case::absent_stream("{}", "False")]
fn streaming_is_written_as_python_text(#[case] optional: &str, #[case] expected: &str) {
    let kwargs = map(&format!(
        r#"{{"optional_params": {optional}, "standard_logging_object": {{"call_type": "acompletion"}}}}"#
    ));
    let attributes = request_attributes(&config(false), &kwargs, None);
    assert_eq!(
        attribute(&attributes, "llm.is_streaming"),
        Some(&Value::from(expected.to_owned()))
    );
}

#[rstest]
#[case::zero_max_tokens(r#"{"max_tokens": 0, "temperature": 0, "top_p": 0}"#)]
#[case::null_values(r#"{"max_tokens": null, "temperature": null, "top_p": null}"#)]
#[case::absent_values("{}")]
fn sampling_parameters_are_written_only_when_truthy(#[case] optional: &str) {
    let kwargs = map(&format!(
        r#"{{"optional_params": {optional}, "standard_logging_object": {{"call_type": "acompletion"}}}}"#
    ));
    let attributes = request_attributes(&config(false), &kwargs, None);
    assert!(!keys(&attributes).contains(&"gen_ai.request.max_tokens"));
    assert!(!keys(&attributes).contains(&"gen_ai.request.temperature"));
    assert!(!keys(&attributes).contains(&"gen_ai.request.top_p"));
}

#[test]
fn the_response_id_falls_back_to_the_payload_id() {
    let kwargs =
        map(r#"{"standard_logging_object": {"call_type": "acompletion", "id": "payload-id"}}"#);
    let without_id = value(r#"{"model": "gpt-4o"}"#);
    let attributes = request_attributes(&config(false), &kwargs, Some(&without_id));
    assert_eq!(
        attribute(&attributes, "gen_ai.response.id"),
        Some(&Value::from("payload-id"))
    );

    let empty_response = value("{}");
    let empty_attributes = request_attributes(&config(false), &kwargs, Some(&empty_response));
    assert_eq!(
        attribute(&empty_attributes, "gen_ai.response.id"),
        Some(&Value::from("payload-id"))
    );

    let with_id = value(r#"{"id": "response-id"}"#);
    let id_attributes = request_attributes(&config(false), &kwargs, Some(&with_id));
    assert_eq!(
        attribute(&id_attributes, "gen_ai.response.id"),
        Some(&Value::from("response-id"))
    );
}

#[test]
fn an_empty_response_keeps_the_request_attributes_and_writes_no_response_model() {
    let kwargs =
        map(r#"{"standard_logging_object": {"call_type": "acompletion", "id": "payload-id"}}"#);
    let empty_response = value("{}");
    let attributes = request_attributes(&config(false), &kwargs, Some(&empty_response));
    assert!(!keys(&attributes).contains(&"gen_ai.response.model"));
    assert_eq!(
        attribute(&attributes, "gen_ai.response.id"),
        Some(&Value::from("payload-id"))
    );
}

#[test]
fn usage_counts_are_written_for_a_billed_inference_call() {
    let kwargs = map(
        r#"{"call_type": "acompletion", "standard_logging_object": {"call_type": "acompletion"}}"#,
    );
    let response =
        value(r#"{"usage": {"total_tokens": 3, "completion_tokens": 1, "prompt_tokens": 2}}"#);
    let attributes = request_attributes(&config(false), &kwargs, Some(&response));
    assert_eq!(
        attribute(&attributes, "gen_ai.usage.total_tokens"),
        Some(&Value::I64(3))
    );
    assert_eq!(
        attribute(&attributes, "gen_ai.usage.output_tokens"),
        Some(&Value::I64(1))
    );
    assert_eq!(
        attribute(&attributes, "gen_ai.usage.input_tokens"),
        Some(&Value::I64(2))
    );
}

#[test]
fn missing_usage_counts_are_written_as_empty_text() {
    let kwargs = map(
        r#"{"call_type": "acompletion", "standard_logging_object": {"call_type": "acompletion"}}"#,
    );
    let response = value(r#"{"usage": {"total_tokens": 3}}"#);
    let attributes = request_attributes(&config(false), &kwargs, Some(&response));
    assert_eq!(
        attribute(&attributes, "gen_ai.usage.output_tokens"),
        Some(&Value::from(""))
    );
    assert_eq!(
        attribute(&attributes, "gen_ai.usage.input_tokens"),
        Some(&Value::from(""))
    );
}

#[test]
fn zero_usage_counts_are_written() {
    let kwargs = map(
        r#"{"call_type": "acompletion", "standard_logging_object": {"call_type": "acompletion"}}"#,
    );
    let response =
        value(r#"{"usage": {"total_tokens": 0, "completion_tokens": 0, "prompt_tokens": 0}}"#);
    let attributes = request_attributes(&config(false), &kwargs, Some(&response));
    assert_eq!(
        attribute(&attributes, "gen_ai.usage.total_tokens"),
        Some(&Value::I64(0))
    );
}

#[test]
fn an_empty_usage_object_writes_no_counts() {
    let kwargs = map(
        r#"{"call_type": "acompletion", "standard_logging_object": {"call_type": "acompletion"}}"#,
    );
    let response = value(r#"{"usage": {}}"#);
    let attributes = request_attributes(&config(false), &kwargs, Some(&response));
    assert!(!keys(&attributes).contains(&"gen_ai.usage.total_tokens"));
}

#[rstest]
#[case::management_call(map(
    r#"{"call_type": "get_responses", "standard_logging_object": {"call_type": "get_responses"}}"#
))]
#[case::other_origin(map(
    r#"{"call_type": "get_responses",
        "litellm_params": {"metadata": {"internal_call_origin": "autorouter_classifier"}},
        "standard_logging_object": {"call_type": "get_responses"}}"#
))]
#[case::metadata_origin_wins_over_litellm_metadata(map(
    r#"{"call_type": "get_responses",
        "litellm_params": {"metadata": {"internal_call_origin": "autorouter_classifier"},
                           "litellm_metadata": {"internal_call_origin": "background_response_cost_poll"}},
        "standard_logging_object": {"call_type": "get_responses"}}"#
))]
fn usage_is_skipped_for_an_unbilled_management_call(#[case] kwargs: MetadataMap) {
    let response = value(r#"{"usage": {"total_tokens": 3}}"#);
    let attributes = request_attributes(&config(false), &kwargs, Some(&response));
    assert!(!keys(&attributes).contains(&"gen_ai.usage.total_tokens"));
}

#[test]
fn usage_is_kept_for_a_background_response() {
    let kwargs = map(
        r#"{"call_type": "get_responses", "standard_logging_object": {"call_type": "get_responses"}}"#,
    );
    let response = value(r#"{"background": true, "usage": {"total_tokens": 3}}"#);
    let attributes = request_attributes(&config(false), &kwargs, Some(&response));
    assert_eq!(
        attribute(&attributes, "gen_ai.usage.total_tokens"),
        Some(&Value::I64(3))
    );
}

#[rstest]
#[case::metadata(r#"{"metadata": {"internal_call_origin": "background_response_cost_poll"}}"#)]
#[case::litellm_metadata(
    r#"{"metadata": {"other": "value"}, "litellm_metadata": {"internal_call_origin": "background_response_cost_poll"}}"#
)]
fn usage_is_kept_for_a_cost_poll_origin(#[case] litellm_params: &str) {
    let kwargs = map(&format!(
        r#"{{"call_type": "get_responses", "litellm_params": {litellm_params},
            "standard_logging_object": {{"call_type": "get_responses"}}}}"#
    ));
    let response = value(r#"{"usage": {"total_tokens": 3}}"#);
    let attributes = request_attributes(&config(false), &kwargs, Some(&response));
    assert_eq!(
        attribute(&attributes, "gen_ai.usage.total_tokens"),
        Some(&Value::I64(3))
    );
}

#[test]
fn the_call_type_for_the_unbilled_test_comes_from_kwargs_not_the_payload() {
    let kwargs = map(
        r#"{"call_type": "get_responses", "standard_logging_object": {"call_type": "acompletion"}}"#,
    );
    let response = value(r#"{"usage": {"total_tokens": 3}}"#);
    let attributes = request_attributes(&config(false), &kwargs, Some(&response));
    assert!(!keys(&attributes).contains(&"gen_ai.usage.total_tokens"));
    assert_eq!(
        attribute(&attributes, "llm.request.type"),
        Some(&Value::from("acompletion"))
    );
}

#[test]
fn an_absent_call_type_writes_an_empty_request_type() {
    let kwargs = map(r#"{"standard_logging_object": {}}"#);
    let attributes = request_attributes(&config(false), &kwargs, None);
    assert_eq!(
        attribute(&attributes, "llm.request.type"),
        Some(&Value::from(""))
    );
}
