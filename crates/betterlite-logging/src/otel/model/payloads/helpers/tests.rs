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
fn guardrail_mode_preserves_mapping() {
    let mode = typed(r#"{"default":"pre_call","tags":{"region":"west"}}"#);
    assert_eq!(
        guardrail_mode_str(&mode).as_deref(),
        Some(r#"{"default":"pre_call","tags":{"region":"west"}}"#)
    );
}

#[test]
fn total_masked_entities_sums_values() {
    assert_eq!(
        total_masked_entities(&typed("{\"email\":2,\"phone\":3}")),
        Some(5)
    );
}

#[rstest::rstest]
#[case::non_integer_counts(r#"{"email":2,"string":"3","fraction":1.5}"#, Some(2))]
#[case::scalar_zero("0", Some(0))]
#[case::empty_map("{}", None)]
fn masked_counts_ignore_non_integers(#[case] raw: &str, #[case] expected: Option<i64>) {
    assert_eq!(total_masked_entities(&typed(raw)), expected);
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

#[rstest::rstest]
#[case::negative_before_zero("[-1,0]", None)]
#[case::zero_before_negative("[0,-1]", Some(0))]
#[case::invalid_before_zero("[true,0]", None)]
#[case::invalid_after_zero("[0,true]", Some(0))]
#[case::first_positive("[null,3,4]", Some(3))]
#[case::positive_after_invalid("[-1,0,5]", Some(5))]
#[case::missing("[null,null]", None)]
fn cache_source_priority(#[case] raw: &str, #[case] expected: Option<i64>) {
    assert_eq!(cache_token_value(typed(raw).as_array().unwrap()), expected);
}

#[test]
fn helpers_preserve_order_and_strings() {
    assert_eq!(dicts(&typed(r#"[{},1,{"a":2}]"#)).len(), 2);
    assert!(dicts(&typed("{}")).is_empty());
    assert_eq!(
        json_or_none(&typed(r#""plain text""#)).as_deref(),
        Some("plain text")
    );
    assert_eq!(json_or_none(&typed("null")).as_deref(), Some("null"));
    assert_eq!(guardrail_mode_str(&typed("null")), None);
    assert_eq!(guardrail_mode_str(&typed("[]")), None);
    assert_eq!(
        guardrail_mode_str(&typed(r#""pre_call""#)).as_deref(),
        Some("pre_call")
    );
}
