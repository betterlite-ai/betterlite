#![cfg(test)]

use super::*;

fn typed(value: &str) -> OtelModelValue {
    serde_json::from_str(value).unwrap()
}

#[test]
fn cache_token_value_prefers_positive() {
    assert_eq!(cache_token_value(&[typed("0"), typed("3")]), Some(3));
    assert_eq!(cache_token_value(&[typed("0")]), Some(0));
    assert_eq!(cache_token_value(&[typed("null")]), None);
}

#[test]
fn guardrail_mode_str_joins_strings() {
    assert_eq!(
        guardrail_mode_str(&typed("[\"pre_call\",\"post_call\"]")).as_deref(),
        Some("pre_call,post_call")
    );
}

#[test]
fn total_masked_entities_sums_values() {
    assert_eq!(
        total_masked_entities(&typed("{\"email\":2,\"phone\":3}")),
        Some(5)
    );
}

#[test]
fn dicts_keeps_object_items() {
    assert_eq!(dicts(&typed("[{\"a\":1},null]")).len(), 1);
}

#[test]
fn json_or_none_serializes() {
    assert_eq!(
        json_or_none(&typed("{\"a\":1}")).as_deref(),
        Some("{\"a\":1}")
    );
}
