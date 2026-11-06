#![cfg(test)]

use std::collections::BTreeSet;

use rstest::rstest;

use super::{
    ANTHROPIC_PARAMETER_NAMES, CHAT_PARAMETER_NAMES, CONTENT_PARAMETER_NAMES,
    EMBEDDING_PARAMETER_NAMES, EXCLUDED_PARAMETER_NAMES, ModelParameters,
    PROVIDER_CHAT_PARAMETER_NAMES, RERANK_PARAMETER_NAMES, RESPONSES_PARAMETER_NAMES,
    TEXT_PARAMETER_NAMES, TRANSCRIPTION_PARAMETER_NAMES, api_parameter_names,
    logging_model_parameters, logging_parameter_names,
};

fn projected(source: &str) -> ModelParameters {
    let parameters: ModelParameters = serde_json::from_str(source).unwrap();
    logging_model_parameters(&parameters)
}

fn keys(parameters: &ModelParameters) -> Vec<&str> {
    parameters.keys().map(String::as_str).collect()
}

fn pinned_parameter_counts() {
    assert_eq!(CHAT_PARAMETER_NAMES.len(), 38);
    assert_eq!(TEXT_PARAMETER_NAMES.len(), 18);
    assert_eq!(EMBEDDING_PARAMETER_NAMES.len(), 5);
    assert_eq!(TRANSCRIPTION_PARAMETER_NAMES.len(), 14);
    assert_eq!(RERANK_PARAMETER_NAMES.len(), 12);
    assert_eq!(RESPONSES_PARAMETER_NAMES.len(), 31);
    assert_eq!(ANTHROPIC_PARAMETER_NAMES.len(), 25);
}

fn pinned_projection_matches_the_fixture() {
    let fixture: BTreeSet<&str> = include_str!("fixtures/logging-names.txt").lines().collect();
    assert_eq!(fixture.len(), 82);
    assert_eq!(logging_parameter_names(), fixture);
}

fn content_parameters_are_dropped() {
    let parameters = projected(
        r#"{"messages":[{"role":"user","content":"hi"}],"prompt":"p","input":"i","system":"s","temperature":0}"#,
    );
    assert_eq!(keys(&parameters), ["temperature"]);
    assert_eq!(parameters["temperature"].get(), "0");
}

fn metadata_parameters_are_dropped() {
    let parameters =
        projected(r#"{"metadata":{"trace":"x"},"litellm_metadata":{"trace":"x"},"model":"gpt-5"}"#);
    assert_eq!(keys(&parameters), ["model"]);
    assert_eq!(parameters["model"].get(), r#""gpt-5""#);
}

fn unknown_parameters_are_dropped() {
    let parameters =
        projected(r#"{"vendor_extension":{"a":1},"custom_parameter":true,"tools":[]}"#);
    assert_eq!(keys(&parameters), ["tools"]);
    assert_eq!(parameters["tools"].get(), "[]");
}

fn null_values_are_copied() {
    let parameters = projected(r#"{"moderation":null,"metadata":null}"#);
    assert_eq!(keys(&parameters), ["moderation"]);
    assert_eq!(parameters["moderation"].get(), "null");
}

fn each_request_type_keeps_its_discriminating_parameter() {
    let parameters = projected(
        r#"{"thinking":{"type":"enabled"},"best_of":2,"dimensions":3,"chunking_strategy":"auto","documents":["a"],"background":true,"stop_sequences":["end"]}"#,
    );
    assert_eq!(keys(&parameters).len(), 7);
    assert_eq!(parameters["thinking"].get(), r#"{"type":"enabled"}"#);
    assert_eq!(parameters["best_of"].get(), "2");
    assert_eq!(parameters["dimensions"].get(), "3");
    assert_eq!(parameters["chunking_strategy"].get(), r#""auto""#);
    assert_eq!(parameters["documents"].get(), r#"["a"]"#);
    assert_eq!(parameters["background"].get(), "true");
    assert_eq!(parameters["stop_sequences"].get(), r#"["end"]"#);
}

fn each_request_type_declares_its_discriminating_parameter() {
    assert!(CHAT_PARAMETER_NAMES.contains(&"thinking"));
    assert!(TEXT_PARAMETER_NAMES.contains(&"best_of"));
    assert!(EMBEDDING_PARAMETER_NAMES.contains(&"dimensions"));
    assert!(TRANSCRIPTION_PARAMETER_NAMES.contains(&"chunking_strategy"));
    assert!(RERANK_PARAMETER_NAMES.contains(&"documents"));
    assert!(RESPONSES_PARAMETER_NAMES.contains(&"background"));
    assert!(ANTHROPIC_PARAMETER_NAMES.contains(&"stop_sequences"));
}

fn content_names_are_the_four_lite_llm_omits() {
    assert_eq!(
        CONTENT_PARAMETER_NAMES,
        &["input", "messages", "prompt", "system"]
    );
}

fn excluded_names_are_metadata_and_litellm_metadata() {
    assert_eq!(EXCLUDED_PARAMETER_NAMES, &["litellm_metadata", "metadata"]);
}

fn provider_chat_names_are_thinking() {
    assert_eq!(PROVIDER_CHAT_PARAMETER_NAMES, &["thinking"]);
}

fn api_names_keep_content_and_drop_metadata() {
    let names = api_parameter_names();
    assert_eq!(names.len(), 86);
    assert!(names.contains("input"));
    assert!(names.contains("messages"));
    assert!(names.contains("prompt"));
    assert!(names.contains("system"));
    assert!(!names.contains("metadata"));
    assert!(!names.contains("litellm_metadata"));
}

fn logging_names_drop_the_content_names() {
    let api = api_parameter_names();
    let logging = logging_parameter_names();
    assert_eq!(logging.len(), 82);
    assert!(!logging.contains("input"));
    assert!(!logging.contains("messages"));
    assert!(!logging.contains("prompt"));
    assert!(!logging.contains("system"));
    assert_eq!(
        api.difference(&logging).copied().collect::<BTreeSet<_>>(),
        CONTENT_PARAMETER_NAMES.iter().copied().collect()
    );
}

#[rstest]
#[case::pinned_parameter_counts(pinned_parameter_counts)]
#[case::pinned_projection_matches_the_fixture(pinned_projection_matches_the_fixture)]
#[case::content_parameters_are_dropped(content_parameters_are_dropped)]
#[case::metadata_parameters_are_dropped(metadata_parameters_are_dropped)]
#[case::unknown_parameters_are_dropped(unknown_parameters_are_dropped)]
#[case::null_values_are_copied(null_values_are_copied)]
#[case::each_request_type_keeps_its_discriminating_parameter(
    each_request_type_keeps_its_discriminating_parameter
)]
#[case::each_request_type_declares_its_discriminating_parameter(
    each_request_type_declares_its_discriminating_parameter
)]
#[case::content_names_are_the_four_lite_llm_omits(content_names_are_the_four_lite_llm_omits)]
#[case::excluded_names_are_metadata_and_litellm_metadata(
    excluded_names_are_metadata_and_litellm_metadata
)]
#[case::provider_chat_names_are_thinking(provider_chat_names_are_thinking)]
#[case::api_names_keep_content_and_drop_metadata(api_names_keep_content_and_drop_metadata)]
#[case::logging_names_drop_the_content_names(logging_names_drop_the_content_names)]
fn parameter_projections_behavior_tests_behavior(#[case] scenario: fn()) {
    scenario();
}
