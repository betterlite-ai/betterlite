#![cfg(test)]

use super::*;

#[test]
fn cache_token_value_prefers_positive() {
    assert_eq!(
        cache_token_value(&[Value::from(0), Value::from(3)]),
        Some(3)
    );
    assert_eq!(cache_token_value(&[Value::from(0)]), Some(0));
    assert_eq!(cache_token_value(&[Value::Null]), None);
}

#[test]
fn guardrail_mode_str_joins_strings() {
    assert_eq!(
        guardrail_mode_str(&serde_json::json!(["pre_call", "post_call"])).as_deref(),
        Some("pre_call,post_call")
    );
}

#[test]
fn total_masked_entities_sums_values() {
    assert_eq!(
        total_masked_entities(&serde_json::json!({"email": 2, "phone": 3})),
        Some(5)
    );
}

#[test]
fn dicts_keeps_object_items() {
    assert_eq!(dicts(&serde_json::json!([{"a": 1}, null])).len(), 1);
}

#[test]
fn json_or_none_serializes() {
    assert_eq!(
        json_or_none(&serde_json::json!({"a": 1})).as_deref(),
        Some("{\"a\":1}")
    );
}
