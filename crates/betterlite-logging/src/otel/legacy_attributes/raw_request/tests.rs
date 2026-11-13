#![cfg(test)]

use std::fs;
use std::path::PathBuf;
use std::str;

use opentelemetry::Value;
use opentelemetry::trace::{Span as _, Tracer as _, TracerProvider as _};
use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider, Span};
use rstest::rstest;

use super::{parse_raw_response, set_raw_request_attributes};
use crate::otel::model::metadata::wire::MetadataMap;

fn map(text: &str) -> MetadataMap {
    serde_json::from_str(text).expect("map")
}

fn raw_request_attributes(kwargs: &MetadataMap) -> Vec<(String, Value)> {
    let exporter = InMemorySpanExporter::default();
    let provider = SdkTracerProvider::builder()
        .with_simple_exporter(exporter.clone())
        .build();
    let mut span: Span = provider.tracer("b3856-raw-request").start("case");
    set_raw_request_attributes(&mut span, kwargs);
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

#[test]
fn request_entries_use_the_provider_prefix() {
    let kwargs = map(r#"{
            "litellm_params": {"custom_llm_provider": "anthropic"},
            "additional_args": {"complete_input_dict": {"model": "claude-3", "max_tokens": 1024, "stream": true}}
        }"#);
    let attributes = raw_request_attributes(&kwargs);
    assert_eq!(
        keys(&attributes),
        vec![
            "llm.anthropic.max_tokens",
            "llm.anthropic.model",
            "llm.anthropic.stream",
        ]
    );
    assert_eq!(
        attribute(&attributes, "llm.anthropic.max_tokens"),
        Some(&Value::I64(1024))
    );
}

#[rstest]
#[case::absent_provider(
    r#"{"additional_args": {"complete_input_dict": {"model": "m"}}}"#,
    "llm.Unknown.model"
)]
#[case::null_provider(
    r#"{"litellm_params": {"custom_llm_provider": null}, "additional_args": {"complete_input_dict": {"model": "m"}}}"#,
    "llm.None.model"
)]
fn the_provider_label_falls_back_to_unknown_and_none(#[case] raw: &str, #[case] expected: &str) {
    let attributes = raw_request_attributes(&map(raw));
    assert_eq!(attribute(&attributes, expected), Some(&Value::from("m")));
}

#[test]
fn response_entries_follow_request_entries_and_replace_the_same_key() {
    let kwargs = map(r#"{
            "litellm_params": {"custom_llm_provider": "anthropic"},
            "additional_args": {"complete_input_dict": {"model": "claude-3", "max_tokens": 1024}},
            "original_response": "{\"model\": \"claude-3-5\", \"stop_reason\": \"end_turn\"}"
        }"#);
    let attributes = raw_request_attributes(&kwargs);
    assert_eq!(
        keys(&attributes),
        vec![
            "llm.anthropic.max_tokens",
            "llm.anthropic.model",
            "llm.anthropic.stop_reason",
        ]
    );
    assert_eq!(
        attribute(&attributes, "llm.anthropic.model"),
        Some(&Value::from("claude-3-5"))
    );
}

#[test]
fn non_json_text_is_written_as_the_stringified_raw_response() {
    let kwargs = map(r#"{
            "litellm_params": {"custom_llm_provider": "anthropic"},
            "original_response": "not json at all"
        }"#);
    let attributes = raw_request_attributes(&kwargs);
    assert_eq!(
        attribute(&attributes, "llm.anthropic.stringified_raw_response"),
        Some(&Value::from("not json at all"))
    );
}

#[rstest]
#[case::array("[1, 2]")]
#[case::number("5")]
#[case::scalar_text("\"just a string\"")]
#[case::null("null")]
fn json_that_is_not_an_object_keeps_the_request_entries_and_adds_nothing(
    #[case] raw_response: &str,
) {
    let kwargs = map(&format!(
        r#"{{"litellm_params": {{"custom_llm_provider": "anthropic"}}, "additional_args": {{"complete_input_dict": {{"model": "claude-3"}}}}, "original_response": {}}}"#,
        serde_json::to_string(raw_response).expect("encoded response")
    ));
    let attributes = raw_request_attributes(&kwargs);
    assert_eq!(keys(&attributes), vec!["llm.anthropic.model"]);
    assert_eq!(
        attribute(&attributes, "llm.anthropic.model"),
        Some(&Value::from("claude-3"))
    );
}

#[test]
fn an_object_response_with_no_entries_writes_nothing() {
    let kwargs = map(
        r#"{"litellm_params": {"custom_llm_provider": "anthropic"}, "original_response": "{}"}"#,
    );
    assert!(raw_request_attributes(&kwargs).is_empty());
}

#[rstest]
#[case::empty_dict(r#"{"additional_args": {"complete_input_dict": {}}}"#)]
#[case::null_dict(r#"{"additional_args": {"complete_input_dict": null}}"#)]
#[case::absent_args("{}")]
fn a_missing_or_empty_request_dict_writes_nothing(#[case] raw: &str) {
    assert!(raw_request_attributes(&map(raw)).is_empty());
}

#[rstest]
#[case::empty_text(r#"{"original_response": ""}"#)]
#[case::object_response(r#"{"original_response": {"model": "m"}}"#)]
fn an_unusable_raw_response_writes_nothing(#[case] raw: &str) {
    assert!(raw_request_attributes(&map(raw)).is_empty());
}

#[rstest]
#[case::broken_object("{\"model\": }")]
#[case::trailing_garbage("{\"model\": \"m\"} tail")]
#[case::empty("")]
fn parse_raw_response_returns_none_for_unparsable_text(#[case] text: &str) {
    assert!(parse_raw_response(text).is_none());
}

#[test]
fn parse_raw_response_returns_the_parsed_value() {
    let parsed = parse_raw_response("{\"model\": \"m\"}").expect("parsed");
    assert!(parsed.as_object().is_some_and(|fields| fields.len() == 1));
    assert!(parse_raw_response("[1]").is_some());
}

#[rstest]
fn replays_the_legacy_raw_response_corpus(
    #[files("fuzz/corpus/legacy_raw_response/*")] path: PathBuf,
) {
    let bytes = fs::read(path).expect("corpus file");
    if let Ok(text) = str::from_utf8(&bytes) {
        drop(parse_raw_response(text));
    }
}
