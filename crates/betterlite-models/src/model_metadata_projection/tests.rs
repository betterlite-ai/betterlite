#![cfg(test)]

use super::project;
use rstest::rstest;
use serde_json::value::RawValue;
use std::collections::BTreeMap;

fn decode(source: &str) -> BTreeMap<String, Box<RawValue>> {
    let raw: &RawValue = serde_json::from_str(source).unwrap();
    serde_json::from_str(
        project("catalog/model", "model", "fixture", raw)
            .unwrap()
            .get(),
    )
    .unwrap()
}

#[rstest]
#[case::metadata_defaults(r#"{"mode":null}"#)]
#[case::metadata_field_projection(r#"{"supports_vision":false,"supported_endpoints":[],"provider_specific_entry":{},"input_cost_per_token":0}"#)]
#[case::threshold_cost_extensions(r#"{"input_cost_per_token_above_200k_tokens":null,"input_cost_per_token_above_999k_tokens":1,"input_cost_per_token_above_999_tokens_priority":2,"ordinary":3}"#)]
fn model_metadata_projection_cases(#[case] source: &str) {
    let projected = decode(source);
    assert_eq!(projected["key"].get(), r#""catalog/model""#);
    assert_eq!(projected["litellm_provider"].get(), r#""fixture""#);
}

#[test]
fn defaults_prices_but_preserves_explicit_null_provider() {
    let raw: &RawValue = serde_json::from_str(
        r#"{"input_cost_per_token":null,"output_cost_per_token":null,"litellm_provider":null}"#,
    )
    .unwrap();
    let projected: BTreeMap<String, Box<RawValue>> =
        serde_json::from_str(project("model", "model", "openai", raw).unwrap().get()).unwrap();
    assert_eq!(projected["input_cost_per_token"].get(), "0");
    assert_eq!(projected["output_cost_per_token"].get(), "0");
    assert_eq!(projected["litellm_provider"].get(), "null");
}

#[test]
fn threshold_suffixes_are_copied_without_priority_extensions() {
    let raw: &RawValue = serde_json::from_str(
        r#"{"input_cost_per_token_above_999k_tokens":null,"input_cost_per_token_above_999_tokens_priority":2,"unknown":3}"#,
    )
    .unwrap();
    let projected: BTreeMap<String, Box<RawValue>> =
        serde_json::from_str(project("model", "model", "openai", raw).unwrap().get()).unwrap();
    assert_eq!(
        projected["input_cost_per_token_above_999k_tokens"].get(),
        "null"
    );
    assert!(!projected.contains_key("input_cost_per_token_above_999_tokens_priority"));
    assert!(!projected.contains_key("unknown"));
}

#[test]
fn rejects_non_numeric_non_null_prices() {
    let raw: &RawValue = serde_json::from_str(r#"{"input_cost_per_token":"invalid"}"#).unwrap();
    assert!(project("model", "model", "openai", raw).err().is_some());
}

#[test]
fn omits_threshold_keys_without_digits() {
    let raw: &RawValue = serde_json::from_str(r#"{"cost_above__tokens":1}"#).unwrap();
    let projected: BTreeMap<String, Box<RawValue>> =
        serde_json::from_str(project("model", "model", "openai", raw).unwrap().get()).unwrap();
    assert!(!projected.contains_key("cost_above__tokens"));
}
