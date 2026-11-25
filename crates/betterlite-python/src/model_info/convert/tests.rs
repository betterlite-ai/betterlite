#![cfg(test)]

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use secrecy::ExposeSecret;
use serde_json::value::RawValue;

use super::{
    ModelQuery, ProviderQuery, ReloadEnvelope, ReloadQuery, RuntimeEntries, parse, reload_result,
};
use crate::model_info::error::ModelInfoError;

#[test]
fn model_info_request_parses_every_field() {
    let query: ModelQuery = parse(
        r#"{"model":"gpt-4","custom_llm_provider":"openai","api_base":"https://example.test","api_key":"sk-test"}"#,
    )
    .unwrap();
    assert_eq!(query.model, "gpt-4");
    assert_eq!(query.custom_llm_provider.as_deref(), Some("openai"));
    assert_eq!(query.api_base.as_deref(), Some("https://example.test"));
    assert_eq!(
        query.api_key.as_ref().map(ExposeSecret::expose_secret),
        Some("sk-test")
    );
}

#[test]
fn model_info_request_defaults_the_optional_fields() {
    let query: ModelQuery = parse(r#"{"model":"gpt-4o"}"#).unwrap();
    assert_eq!(query.model, "gpt-4o");
    assert!(query.custom_llm_provider.is_none());
    assert!(query.api_base.is_none());
    assert!(query.api_key.is_none());
}

#[test]
fn model_info_request_rejects_malformed_text() {
    let error = parse::<ModelQuery>("{not json").unwrap_err();
    assert!(matches!(error, ModelInfoError::Request(_)));
}

#[test]
fn model_info_request_rejects_a_wrong_field_type() {
    let error = parse::<ModelQuery>(r#"{"model":42}"#).unwrap_err();
    assert!(matches!(error, ModelInfoError::Request(_)));
}

#[test]
fn model_info_request_parses_the_provider_query() {
    let query: ProviderQuery = parse(r#"{"model":"grok-3","custom_llm_provider":"xai"}"#).unwrap();
    assert_eq!(query.model, "grok-3");
    assert_eq!(query.custom_llm_provider.as_deref(), Some("xai"));
}

#[test]
fn model_info_request_parses_runtime_entries_and_removals() {
    let entries: RuntimeEntries =
        parse(r#"{"entries":{"a":{"max_tokens":1}},"removed":["b"]}"#).unwrap();
    assert_eq!(entries.entries.len(), 1);
    assert!(entries.entries.contains_key("a"));
    assert_eq!(entries.removed, vec!["b".to_owned()]);
}

#[test]
fn model_info_request_defaults_reload_values() {
    let query: ReloadQuery = parse(r#"{"url":"https://example.test/map.json"}"#).unwrap();
    assert_eq!(query.timeout().unwrap().as_secs(), 5);
    assert_eq!(query.max_attempts, 3);
}

#[test]
fn model_info_request_reads_reload_values() {
    let query: ReloadQuery =
        parse(r#"{"url":"https://example.test/map.json","timeout":2,"max_attempts":7}"#).unwrap();
    assert_eq!(query.timeout().unwrap().as_secs(), 2);
    assert_eq!(query.max_attempts, 7);
}

#[test]
fn model_info_request_rejects_a_negative_timeout() {
    let query: ReloadQuery =
        parse(r#"{"url":"https://example.test/map.json","timeout":-1}"#).unwrap();
    assert!(matches!(
        query.timeout(),
        Err(ModelInfoError::Timeout { .. })
    ));
}

#[test]
fn model_info_result_serializes_the_fields_in_order() {
    let fields: IndexMap<String, Json> =
        serde_json::from_str(r#"{"key":"gpt-4","max_tokens":4096}"#).unwrap();
    let text = serde_json::to_string(&fields).unwrap();
    assert_eq!(text, r#"{"key":"gpt-4","max_tokens":4096}"#);
}

#[test]
fn model_info_result_keeps_a_null_optional_field() {
    let fields: IndexMap<String, Json> =
        serde_json::from_str(r#"{"key":"gpt-4","supported_openai_params":null}"#).unwrap();
    let text = serde_json::to_string(&fields).unwrap();
    assert_eq!(text, r#"{"key":"gpt-4","supported_openai_params":null}"#);
}

#[test]
fn model_info_result_writes_a_reloaded_answer() {
    let mut map: IndexMap<String, Box<RawValue>> = IndexMap::new();
    map.insert(
        "gpt-4".to_owned(),
        RawValue::from_string(r#"{"litellm_provider":"openai"}"#.to_owned()).unwrap(),
    );
    let text = reload_result(&ReloadEnvelope::Reloaded {
        model_cost_map: &map,
        revision: Some("abc"),
        etag: None,
    })
    .unwrap();
    assert_eq!(
        text,
        r#"{"status":"reloaded","model_cost_map":{"gpt-4":{"litellm_provider":"openai"}},"revision":"abc","etag":null}"#
    );
}

#[test]
fn model_info_result_writes_an_unavailable_answer() {
    let text = reload_result(&ReloadEnvelope::Unavailable {
        reason: "HTTP 500 from https://example.test",
    })
    .unwrap();
    assert_eq!(
        text,
        r#"{"status":"unavailable","reason":"HTTP 500 from https://example.test"}"#
    );
}
