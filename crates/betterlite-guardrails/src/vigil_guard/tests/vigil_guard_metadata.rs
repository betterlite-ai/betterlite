use betterlite_guardrails::vigil_guard::vigil_guard_metadata::{MetadataValue, collect_metadata};

use crate::support::object;

#[test]
fn collects_allowlist_fields_in_order() {
    let data = object(
        r#"{"model":"gpt-4o","model_group":"chat","provider":"openai","region":"us-east-1","deployment":"dep","user":"u","user_id":"uid","session_id":"sid","conversation_id":"cid","request_id":"rid","tenant_id":"tid","org_id":"oid","unrelated":"ignored"}"#,
    );
    let collected = collect_metadata(&data, None);
    let keys: Vec<&str> = collected.keys().map(String::as_str).collect();
    assert_eq!(
        keys,
        vec![
            "model",
            "model_group",
            "provider",
            "region",
            "deployment",
            "user",
            "user_id",
            "session_id",
            "conversation_id",
            "request_id",
            "tenant_id",
            "org_id",
        ]
    );
    assert_eq!(
        collected.get("model"),
        Some(&MetadataValue::Text("gpt-4o".to_owned()))
    );
    assert_eq!(
        collected.get("org_id"),
        Some(&MetadataValue::Text("oid".to_owned()))
    );
    assert!(!collected.contains_key("unrelated"));
}

#[test]
fn the_first_source_with_a_value_wins() {
    let data = object(
        r#"{"model":"outer","metadata":{"model":"nested","provider":"nested-provider"},"litellm_metadata":{"model":"deep","region":"deep-region"}}"#,
    );
    let collected = collect_metadata(&data, None);
    assert_eq!(
        collected.get("model"),
        Some(&MetadataValue::Text("outer".to_owned()))
    );
    assert_eq!(
        collected.get("provider"),
        Some(&MetadataValue::Text("nested-provider".to_owned()))
    );
    assert_eq!(
        collected.get("region"),
        Some(&MetadataValue::Text("deep-region".to_owned()))
    );
}

#[test]
fn boolean_values_fall_through_to_the_next_source() {
    let data = object(r#"{"model":true,"metadata":{"model":"nested"}}"#);
    let collected = collect_metadata(&data, None);
    assert_eq!(
        collected.get("model"),
        Some(&MetadataValue::Text("nested".to_owned()))
    );
}

#[test]
fn null_values_are_skipped() {
    let data = object(r#"{"model":null,"metadata":{"model":"nested"}}"#);
    let collected = collect_metadata(&data, None);
    assert_eq!(
        collected.get("model"),
        Some(&MetadataValue::Text("nested".to_owned()))
    );
}

#[test]
fn non_allowlisted_fields_are_ignored() {
    let data = object(r#"{"other":"value","api_key":"secret"}"#);
    assert!(collect_metadata(&data, None).is_empty());
}

#[test]
fn litellm_call_id_is_added_last() {
    let data = object(r#"{"model":"gpt-4o","litellm_call_id":"call-1"}"#);
    let collected = collect_metadata(&data, None);
    let keys: Vec<&str> = collected.keys().map(String::as_str).collect();
    assert_eq!(keys, vec!["model", "litellm_call_id"]);
    assert_eq!(
        collected.get("litellm_call_id"),
        Some(&MetadataValue::Text("call-1".to_owned()))
    );
}

#[test]
fn nested_sources_must_be_objects() {
    let data = object(r#"{"model":"outer","metadata":"not an object"}"#);
    let collected = collect_metadata(&data, None);
    assert_eq!(
        collected.get("model"),
        Some(&MetadataValue::Text("outer".to_owned()))
    );
}
