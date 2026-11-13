#![cfg(test)]

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;

use super::scrub_sensitive_metadata;
use crate::logging_metadata::params::LitellmParams;

#[rstest]
#[case::without_params("{}")]
#[case::without_metadata(r#"{"litellm_metadata": {"a": 1}}"#)]
#[case::without_the_key(r#"{"metadata": {"other": "kept"}}"#)]
fn logging_metadata_behavior_tests_behavior(#[case] encoded: &str) {
    let params: LitellmParams = serde_json::from_str(encoded).expect("params decode");
    let scrubbed = scrub_sensitive_metadata(params.clone());
    assert_eq!(scrubbed, params);
}

#[test]
fn scrub_removes_the_masking_function_from_metadata() {
    let params: LitellmParams =
        serde_json::from_str(r#"{"metadata": {"langfuse_masking_function": "fn", "other": 1}}"#)
            .expect("params decode");
    let scrubbed = scrub_sensitive_metadata(params);
    let metadata = scrubbed.metadata.expect("metadata stays");
    assert_eq!(metadata.len(), 1);
    assert_eq!(
        metadata.get("other"),
        Some(&Json::Number(serde_json::Number::from(1_i32)))
    );
}

#[test]
fn scrub_carries_the_typed_masking_function_slot() {
    let params: LitellmParams = serde_json::from_str(
        r#"{"metadata": {"langfuse_masking_function": "fn"}, "_langfuse_masking_function": {"callable": true}}"#,
    )
    .expect("params decode");
    let scrubbed = scrub_sensitive_metadata(params);
    assert_eq!(
        scrubbed.langfuse_masking_function,
        Some(Json::Object(IndexMap::from([(
            String::from("callable"),
            Json::Bool(true)
        )])))
    );
    let metadata = scrubbed.metadata.expect("metadata stays");
    assert!(!metadata.contains_key("langfuse_masking_function"));
}
