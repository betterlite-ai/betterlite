use std::collections::BTreeMap;

use betterlite_logging::otel::plumbing::metrics::attributes::common_attributes;
use rstest::rstest;

use super::support::kwargs;

#[test]
fn the_common_attributes_name_the_operation_provider_model_and_metadata() {
    let attributes = common_attributes(&kwargs("{}")).expect("attributes");
    let expected: Vec<(String, Option<String>)> = [
        ("gen_ai.operation.name", Some("chat")),
        ("gen_ai.provider.name", Some("openai")),
        ("gen_ai.system", Some("openai")),
        ("gen_ai.request.model", Some("gpt-4o")),
        ("gen_ai.framework", Some("litellm")),
        ("metadata.user_api_key_alias", Some("key-a")),
        ("hidden_params", Some(r#"{"model_id":"deployment-1"}"#)),
    ]
    .into_iter()
    .map(|(key, value)| (key.to_owned(), value.map(str::to_owned)))
    .collect();
    assert_eq!(attributes, expected);
}

#[test]
fn metadata_values_are_written_as_json_and_nulls_are_dropped() {
    let extra = r#"{"standard_logging_object": {"metadata": {
        "requester_metadata": {"team": 1},
        "applied_guardrails": ["a", "b"],
        "requester_ip_address": true,
        "user_api_key_org_id": null,
        "user_api_key_user_id": ""
    }}}"#;
    let attributes = common_attributes(&kwargs(extra)).expect("attributes");
    let by_key: BTreeMap<&str, &str> = attributes
        .iter()
        .filter_map(|(key, value)| value.as_deref().map(|text| (key.as_str(), text)))
        .collect();
    assert_eq!(by_key["metadata.requester_metadata"], r#"{"team":1}"#);
    assert_eq!(by_key["metadata.applied_guardrails"], r#"["a","b"]"#);
    assert_eq!(by_key["metadata.requester_ip_address"], "true");
    assert_eq!(by_key["metadata.user_api_key_user_id"], "");
    assert!(!by_key.contains_key("metadata.user_api_key_org_id"));
}

#[rstest]
#[case::empty_hidden_params(r#"{"standard_logging_object": {"hidden_params": {}}}"#)]
#[case::empty_standard_logging_object(r#"{"standard_logging_object": {}}"#)]
fn a_standard_logging_object_without_hidden_params_makes_no_label(#[case] extra: &str) {
    let attributes = common_attributes(&kwargs(extra)).expect("attributes");
    assert!(!attributes.iter().any(|(key, _)| key == "hidden_params"));
}

#[rstest]
#[case::completion("acompletion", "chat")]
#[case::responses("aresponses", "chat")]
#[case::text_completion("atext_completion", "text_completion")]
#[case::embedding("aembedding", "embeddings")]
#[case::image_generation("aimage_generation", "generate_content")]
#[case::vector_store_search("avector_store_search", "retrieval")]
#[case::unmapped("mystery_call", "chat")]
fn the_operation_name_follows_the_call_type(#[case] call_type: &str, #[case] expected: &str) {
    let extra = format!(r#"{{"call_type": "{call_type}"}}"#);
    let attributes = common_attributes(&kwargs(&extra)).expect("attributes");
    let operation = attributes
        .iter()
        .find(|(key, _)| key == "gen_ai.operation.name")
        .expect("operation");
    assert_eq!(operation.1.as_deref(), Some(expected));
}

#[test]
fn a_call_without_a_call_type_is_a_chat_operation() {
    let attributes = common_attributes(&kwargs(r#"{"call_type": null}"#)).expect("attributes");
    let chat = ("gen_ai.operation.name".to_owned(), Some("chat".to_owned()));
    assert!(attributes.contains(&chat));
}

#[rstest]
#[case::text(r#"{"standard_logging_object": "n/a"}"#)]
#[case::number(r#"{"standard_logging_object": 3}"#)]
#[case::wrong_member_types(
    r#"{"standard_logging_object": {"metadata": "n/a", "hidden_params": 3}}"#
)]
fn a_standard_logging_object_that_is_not_an_object_adds_no_metadata(#[case] extra: &str) {
    let attributes = common_attributes(&kwargs(extra)).expect("attributes");
    let keys: Vec<&str> = attributes.iter().map(|(key, _)| key.as_str()).collect();
    assert!(!keys.iter().any(|key| key.starts_with("metadata.")));
    assert!(!keys.contains(&"hidden_params"));
}
