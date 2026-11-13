#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};

use opentelemetry::trace::{Span as _, Tracer as _, TracerProvider as _};
use opentelemetry::{Array, StringValue, Value};
use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider, Span};
use rstest::rstest;

use super::{
    REQUEST_STREAM, gen_ai_operation_name, semconv_latest_experimental, set_cache_token_attributes,
    set_semconv_request_attributes,
};
use crate::otel::legacy_policy::config::{Exporter, OpenTelemetryConfig, Signals};
use crate::otel::model::utils::coercion::OtelModelValue;
use crate::otel::vocabulary::semconv::OTELSemconvCategory;

fn params(text: &str) -> BTreeMap<String, OtelModelValue> {
    serde_json::from_str(text).expect("params")
}

fn base_config() -> OpenTelemetryConfig {
    OpenTelemetryConfig {
        exporter: Exporter::Console,
        endpoint: None,
        headers: None,
        signals: Signals::default(),
        service_name: String::new(),
        deployment_environment: String::new(),
        model_id: String::new(),
        ignore_context_propagation: false,
        skip_set_global: false,
        capture_message_content: None,
        semconv_stability_opt_in: BTreeSet::new(),
        baggage_team_metadata_keys: Vec::new(),
        baggage_metadata_keys: Vec::new(),
        attributes: None,
    }
}

fn record_attributes<F>(write: F) -> Vec<(String, Value)>
where
    F: FnOnce(&mut Span),
{
    let exporter = InMemorySpanExporter::default();
    let provider = SdkTracerProvider::builder()
        .with_simple_exporter(exporter.clone())
        .build();
    let mut span = provider.tracer("legacy-policy-tests").start("test");
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

#[rstest]
#[case::embedding(Some("embedding"), "embeddings")]
#[case::aembedding(Some("aembedding"), "embeddings")]
#[case::text_completion(Some("text_completion"), "text_completion")]
#[case::atext_completion(Some("atext_completion"), "text_completion")]
#[case::completion(Some("completion"), "chat")]
#[case::both(Some("embedding_text_completion"), "embeddings")]
#[case::missing(None, "chat")]
#[case::empty(Some(""), "chat")]
fn operation_names_follow_litellm(#[case] call_type: Option<&str>, #[case] expected: &str) {
    assert_eq!(gen_ai_operation_name(call_type), expected);
}

#[test]
fn sampling_attributes_include_zero_values() {
    let optional = params(
        r#"{"frequency_penalty": 0, "presence_penalty": 0.5, "top_k": 0, "seed": 0, "temperature": 1.0}"#,
    );
    let attributes = record_attributes(|span| {
        set_semconv_request_attributes(span, &optional);
    });
    assert_eq!(
        attributes,
        vec![
            ("gen_ai.request.frequency_penalty".to_owned(), Value::I64(0)),
            (
                "gen_ai.request.presence_penalty".to_owned(),
                Value::F64(0.5)
            ),
            ("gen_ai.request.top_k".to_owned(), Value::I64(0)),
            ("gen_ai.request.seed".to_owned(), Value::I64(0)),
        ]
    );
}

#[test]
fn null_sampling_attributes_are_skipped() {
    let optional = params(r#"{"frequency_penalty": null, "seed": null}"#);
    let attributes = record_attributes(|span| {
        set_semconv_request_attributes(span, &optional);
    });
    assert!(attributes.is_empty());
}

#[rstest]
#[case::single_string(r#"{"stop": "END"}"#, vec![StringValue::from("END")])]
#[case::list_strings(r#"{"stop": ["END", "STOP"]}"#, vec![StringValue::from("END"), StringValue::from("STOP")])]
#[case::mixed_list(r#"{"stop": ["END", 5, true]}"#, vec![StringValue::from("END"), StringValue::from("5"), StringValue::from("True")])]
#[case::empty_list(r#"{"stop": []}"#, Vec::new())]
fn stop_sequences_become_a_string_array(#[case] raw: &str, #[case] expected: Vec<StringValue>) {
    let optional = params(raw);
    let attributes = record_attributes(|span| {
        set_semconv_request_attributes(span, &optional);
    });
    assert_eq!(
        attributes,
        vec![(
            "gen_ai.request.stop_sequences".to_owned(),
            Value::Array(Array::from(expected))
        )]
    );
}

#[test]
fn a_null_stop_writes_no_sequence_attribute() {
    let optional = params(r#"{"stop": null}"#);
    let attributes = record_attributes(|span| {
        set_semconv_request_attributes(span, &optional);
    });
    assert!(attributes.is_empty());
}

#[rstest]
#[case::true_value(r#"{"stream": true}"#, true)]
#[case::false_value(r#"{"stream": false}"#, false)]
#[case::missing("{}", false)]
#[case::text(r#"{"stream": "true"}"#, false)]
fn stream_is_written_only_when_true(#[case] raw: &str, #[case] written: bool) {
    let optional = params(raw);
    let attributes = record_attributes(|span| {
        set_semconv_request_attributes(span, &optional);
    });
    assert_eq!(
        attributes,
        if written {
            vec![(REQUEST_STREAM.to_owned(), Value::Bool(true))]
        } else {
            Vec::new()
        }
    );
}

#[rstest]
#[case::two(r#"{"n": 2}"#, Some(2))]
#[case::three(r#"{"n": 3}"#, Some(3))]
#[case::one(r#"{"n": 1}"#, None)]
#[case::zero(r#"{"n": 0}"#, None)]
#[case::negative(r#"{"n": -2}"#, None)]
#[case::float(r#"{"n": 2.0}"#, None)]
#[case::boolean(r#"{"n": true}"#, None)]
fn choice_count_is_written_above_one(#[case] raw: &str, #[case] count: Option<i64>) {
    let optional = params(raw);
    let attributes = record_attributes(|span| {
        set_semconv_request_attributes(span, &optional);
    });
    let expected = count.map_or_else(Vec::new, |value| {
        vec![("gen_ai.request.choice.count".to_owned(), Value::I64(value))]
    });
    assert_eq!(attributes, expected);
}

#[test]
fn cache_token_attributes_skip_zero_values() {
    let payload = params(
        r#"{"metadata": {"usage_object": {"cache_creation_input_tokens": 0, "cache_read_input_tokens": 100}}}"#,
    );
    let attributes = record_attributes(|span| {
        set_cache_token_attributes(span, &payload);
    });
    assert_eq!(
        attributes,
        vec![(
            "gen_ai.usage.cache_read.input_tokens".to_owned(),
            Value::I64(100)
        )]
    );
}

#[test]
fn cache_token_attributes_are_written_independently() {
    let payload = params(r#"{"metadata": {"usage_object": {"cache_creation_input_tokens": 50}}}"#);
    let attributes = record_attributes(|span| {
        set_cache_token_attributes(span, &payload);
    });
    assert_eq!(
        attributes,
        vec![(
            "gen_ai.usage.cache_creation.input_tokens".to_owned(),
            Value::I64(50)
        )]
    );
}

#[test]
fn cache_token_attributes_need_a_usage_object() {
    let payload = params(r#"{"metadata": {"usage_object": null}}"#);
    let attributes = record_attributes(|span| {
        set_cache_token_attributes(span, &payload);
    });
    assert!(attributes.is_empty());
}

#[test]
fn the_semconv_gate_reads_the_opt_in_set() {
    let mut config = base_config();
    assert!(!semconv_latest_experimental(&config));
    config
        .semconv_stability_opt_in
        .insert(OTELSemconvCategory::GenAiLatestExperimental);
    assert!(semconv_latest_experimental(&config));
}
