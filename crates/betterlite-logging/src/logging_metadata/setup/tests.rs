#![cfg(test)]

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;
use rust_decimal::Decimal;
use std::str::FromStr as _;

use super::StandardMetadataRequest;
use super::get_standard_logging_metadata;
use super::merge_litellm_metadata;
use crate::fields::payload::records::StandardLoggingMetadata;
use crate::logging_metadata::decode::DecodeError;
use crate::logging_metadata::params::LitellmParams;

fn params(encoded: &str) -> LitellmParams {
    serde_json::from_str(encoded).expect("litellm params decode")
}

fn metadata(encoded: &str) -> IndexMap<String, Json> {
    serde_json::from_str(encoded).expect("metadata decodes")
}

fn metadata_request(metadata: Option<&IndexMap<String, Json>>) -> StandardMetadataRequest<'_> {
    StandardMetadataRequest {
        metadata,
        params: None,
        prompt_integration: None,
        applied_guardrails: None,
        mcp_tool_call_metadata: None,
        vector_store_request_metadata: None,
        usage_object: None,
        custom_llm_provider: None,
    }
}

fn record(metadata: &IndexMap<String, Json>) -> StandardLoggingMetadata {
    get_standard_logging_metadata(&metadata_request(Some(metadata)))
        .expect("metadata record decodes")
}

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

#[test]
fn merge_litellm_metadata_prefers_user_metadata_and_drops_auth_objects() {
    let params = params(
        r#"{"metadata": {"team": "user", "user_api_key_auth": {"secret": 1}, "user_api_key_budget_reservation": {"spend": 2}},
            "litellm_metadata": {"team": "model", "model_group": "gpt-4o"}}"#,
    );
    let merged = merge_litellm_metadata(&params);
    assert_eq!(merged.get("team"), Some(&text("user")));
    assert_eq!(merged.get("model_group"), Some(&text("gpt-4o")));
    assert!(!merged.contains_key("user_api_key_auth"));
    assert!(!merged.contains_key("user_api_key_budget_reservation"));
}

#[test]
fn merge_litellm_metadata_filters_only_the_user_side() {
    let params = params(
        r#"{"metadata": {"team": "user"},
            "litellm_metadata": {"user_api_key_auth": {"secret": 1}, "team": "model"}}"#,
    );
    let merged = merge_litellm_metadata(&params);
    assert_eq!(
        merged.get("user_api_key_auth"),
        Some(&Json::Object(IndexMap::from([(
            String::from("secret"),
            Json::Number(serde_json::Number::from(1_i32))
        )])))
    );
    assert_eq!(merged.get("team"), Some(&text("user")));
}

#[rstest]
#[case::both_absent("{}")]
#[case::only_user_metadata(r#"{"metadata": {"a": 1}}"#)]
#[case::only_model_metadata(r#"{"litellm_metadata": {"b": 2}}"#)]
fn logging_metadata_behavior_tests_behavior(#[case] encoded: &str) {
    let params = params(encoded);
    let merged = merge_litellm_metadata(&params);
    let expected_len = match encoded {
        "{}" => 0,
        _ => 1,
    };
    assert_eq!(merged.len(), expected_len);
}

#[test]
fn standard_metadata_copies_declared_keys_and_drops_unknown_ones() {
    let metadata = metadata(
        r#"{"user_api_key_alias": "alias", "user_agent": "agent/1", "unknown": "secret", "user_api_key_spend": 1.5}"#,
    );
    let record = record(&metadata);
    assert_eq!(record.user_api_key_alias.as_deref(), Some("alias"));
    assert_eq!(record.user_agent.as_deref(), Some("agent/1"));
    assert_eq!(
        record.user_api_key_spend.map(|value| value.0),
        Some(Decimal::from_str("1.5").expect("decimal"))
    );
}

#[test]
fn standard_metadata_rejects_a_mistyped_declared_key_and_names_it() {
    let metadata = metadata(r#"{"user_api_key_alias": 7}"#);
    let error =
        get_standard_logging_metadata(&metadata_request(Some(&metadata))).expect_err("wrong type");
    let DecodeError::Field { path, .. } = error else {
        panic!("a field error is expected");
    };
    assert_eq!(path, "user_api_key_alias");
}

#[test]
fn standard_metadata_builds_prompt_management_metadata_from_litellm_params() {
    let metadata = metadata("{}");
    let params = params(r#"{"prompt_id": "prompt-1", "prompt_variables": {"name": "Ada"}}"#);
    let mut request = metadata_request(Some(&metadata));
    request.params = Some(&params);
    request.prompt_integration = Some("langfuse");
    let record = get_standard_logging_metadata(&request).expect("record");
    let Json::Object(prompt) = record.prompt_management_metadata.expect("prompt metadata") else {
        panic!("prompt metadata is an object");
    };
    assert_eq!(prompt.get("prompt_id"), Some(&text("prompt-1")));
    assert_eq!(prompt.get("prompt_integration"), Some(&text("langfuse")));
    assert_eq!(
        prompt.get("prompt_variables"),
        Some(&Json::Object(IndexMap::from([(
            String::from("name"),
            text("Ada")
        )])))
    );
}

#[test]
fn standard_metadata_lets_a_recorded_prompt_management_value_win() {
    let metadata = metadata(r#"{"prompt_management_metadata": {"prompt_id": "recorded"}}"#);
    let params = params(r#"{"prompt_id": "prompt-1"}"#);
    let mut request = metadata_request(Some(&metadata));
    request.params = Some(&params);
    request.prompt_integration = Some("langfuse");
    let record = get_standard_logging_metadata(&request).expect("record");
    let Json::Object(prompt) = record.prompt_management_metadata.expect("prompt metadata") else {
        panic!("prompt metadata is an object");
    };
    assert_eq!(prompt.get("prompt_id"), Some(&text("recorded")));
}

#[test]
fn standard_metadata_ignores_prompt_management_without_an_integration() {
    let metadata = metadata("{}");
    let params = params(r#"{"prompt_id": "prompt-1"}"#);
    let mut request = metadata_request(Some(&metadata));
    request.params = Some(&params);
    assert!(
        get_standard_logging_metadata(&request)
            .expect("record")
            .prompt_management_metadata
            .is_none()
    );
}

#[test]
fn standard_metadata_merges_applied_guardrails_in_order_without_duplicates() {
    let metadata = metadata(r#"{"applied_guardrails": ["b", "c"]}"#);
    let argument = vec!["a".to_owned(), "b".to_owned()];
    let mut request = metadata_request(Some(&metadata));
    request.applied_guardrails = Some(&argument);
    let record = get_standard_logging_metadata(&request).expect("record");
    assert_eq!(
        record.applied_guardrails,
        Some(vec!["a".to_owned(), "b".to_owned(), "c".to_owned()])
    );
}

#[test]
fn standard_metadata_keeps_recorded_guardrails_when_the_argument_is_empty() {
    let metadata = metadata(r#"{"applied_guardrails": ["b"]}"#);
    let mut request = metadata_request(Some(&metadata));
    request.applied_guardrails = Some(&[]);
    let record = get_standard_logging_metadata(&request).expect("record");
    assert_eq!(record.applied_guardrails, Some(vec!["b".to_owned()]));
}

#[test]
fn standard_metadata_uses_the_argument_when_the_record_has_no_guardrails() {
    let metadata = metadata("{}");
    let argument = vec!["a".to_owned()];
    let mut request = metadata_request(Some(&metadata));
    request.applied_guardrails = Some(&argument);
    let record = get_standard_logging_metadata(&request).expect("record");
    assert_eq!(record.applied_guardrails, Some(vec!["a".to_owned()]));
}

#[test]
fn standard_metadata_keeps_duplicate_guardrails_when_the_record_has_none() {
    let metadata = metadata("{}");
    let argument = vec!["a".to_owned(), "a".to_owned()];
    let mut request = metadata_request(Some(&metadata));
    request.applied_guardrails = Some(&argument);
    let record = get_standard_logging_metadata(&request).expect("record");
    assert_eq!(
        record.applied_guardrails,
        Some(vec!["a".to_owned(), "a".to_owned()])
    );
}

#[test]
fn standard_metadata_copies_a_user_api_key_only_when_it_is_a_sha256_digest() {
    let digest = "a".repeat(64);
    let metadata = metadata(&format!(
        r#"{{"user_api_key": "{digest}", "user_api_key_hash": "old"}}"#
    ));
    let record = record(&metadata);
    assert_eq!(record.user_api_key_hash, Some(digest));
}

#[test]
fn standard_metadata_keeps_a_recorded_hash_when_the_key_is_not_a_digest() {
    let metadata = metadata(r#"{"user_api_key": "sk-raw", "user_api_key_hash": "recorded"}"#);
    let record = record(&metadata);
    assert_eq!(record.user_api_key_hash.as_deref(), Some("recorded"));
}

#[test]
fn standard_metadata_takes_requester_metadata_from_the_nested_metadata_object() {
    let metadata = metadata(r#"{"metadata": {"session": "one"}}"#);
    let record = record(&metadata);
    assert_eq!(
        record.requester_metadata,
        Some(Json::Object(IndexMap::from([(
            String::from("session"),
            text("one")
        )])))
    );
}

#[test]
fn standard_metadata_keeps_a_recorded_requester_metadata_value() {
    let metadata =
        metadata(r#"{"requester_metadata": {"recorded": true}, "metadata": {"session": "one"}}"#);
    let record = record(&metadata);
    assert_eq!(
        record.requester_metadata,
        Some(Json::Object(IndexMap::from([(
            String::from("recorded"),
            Json::Bool(true)
        )])))
    );
}

#[rstest]
#[case::litellm_metadata_true_for_anthropic(
    r#"{"litellm_metadata": {"used_client_oauth_token": true}}"#,
    Some("anthropic"),
    Some(true)
)]
#[case::litellm_metadata_true_for_openai(
    r#"{"litellm_metadata": {"used_client_oauth_token": true}}"#,
    Some("openai"),
    Some(false)
)]
#[case::litellm_metadata_false(
    r#"{"litellm_metadata": {"used_client_oauth_token": false}}"#,
    Some("anthropic"),
    Some(false)
)]
#[case::litellm_metadata_non_bool(
    r#"{"litellm_metadata": {"used_client_oauth_token": "yes"}}"#,
    Some("anthropic"),
    None
)]
#[case::nothing_recorded("{}", None, None)]
fn oauth_resolution_reads_litellm_metadata_first(
    #[case] encoded: &str,
    #[case] provider: Option<&str>,
    #[case] expected: Option<bool>,
) {
    let params = params(encoded);
    let mut request = metadata_request(None);
    request.params = Some(&params);
    request.custom_llm_provider = provider;
    let record = get_standard_logging_metadata(&request).expect("record");
    assert_eq!(record.used_client_oauth_token, expected);
}

#[test]
fn oauth_resolution_falls_back_to_the_metadata_argument() {
    let metadata = metadata(r#"{"used_client_oauth_token": true}"#);
    let params = params("{}");
    let mut request = metadata_request(Some(&metadata));
    request.params = Some(&params);
    request.custom_llm_provider = Some("anthropic");
    let record = get_standard_logging_metadata(&request).expect("record");
    assert_eq!(record.used_client_oauth_token, Some(true));
}

#[test]
fn standard_metadata_uses_supplied_usage_and_tool_metadata_when_the_record_has_none() {
    let metadata = metadata("{}");
    let usage = Json::Object(IndexMap::from([(
        String::from("service_tier"),
        text("priority"),
    )]));
    let tools = Json::Object(IndexMap::from([(String::from("name"), text("search"))]));
    let vector = vec![Json::Object(IndexMap::new())];
    let mut request = metadata_request(Some(&metadata));
    request.usage_object = Some(&usage);
    request.mcp_tool_call_metadata = Some(&tools);
    request.vector_store_request_metadata = Some(&vector);
    let record = get_standard_logging_metadata(&request).expect("record");
    assert_eq!(record.usage_object, Some(usage));
    assert_eq!(record.mcp_tool_call_metadata, Some(tools));
    assert_eq!(record.vector_store_request_metadata, Some(vector));
}

#[test]
fn standard_metadata_lets_a_recorded_usage_object_win() {
    let metadata = metadata(r#"{"usage_object": {"service_tier": "recorded"}}"#);
    let usage = Json::Object(IndexMap::from([(
        String::from("service_tier"),
        text("argument"),
    )]));
    let mut request = metadata_request(Some(&metadata));
    request.usage_object = Some(&usage);
    let record = get_standard_logging_metadata(&request).expect("record");
    assert_eq!(
        record.usage_object,
        Some(Json::Object(IndexMap::from([(
            String::from("service_tier"),
            text("recorded")
        )])))
    );
}

#[test]
fn standard_metadata_fills_prompt_and_oauth_without_a_record() {
    let params = params(
        r#"{"prompt_id": "prompt-1", "litellm_metadata": {"used_client_oauth_token": true}}"#,
    );
    let mut request = metadata_request(None);
    request.params = Some(&params);
    request.prompt_integration = Some("langfuse");
    request.custom_llm_provider = Some("anthropic");
    let record = get_standard_logging_metadata(&request).expect("record");
    assert!(record.prompt_management_metadata.is_some());
    assert_eq!(record.used_client_oauth_token, Some(true));
}
