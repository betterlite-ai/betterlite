#![cfg(test)]

use betterlite_core::agents::json::AgentJson;
use rstest::rstest;
use serde_json::json;

use super::safe_json_text;

fn text(value: AgentJson, max_depth: usize) -> String {
    safe_json_text(&value, max_depth)
}

#[test]
fn safe_json_text_strips_nul_from_values() {
    let stored = text(
        json!({"api_key": "sk-\u{0}live", "nested": ["a\u{0}b"]}),
        100,
    );
    assert_eq!(stored, r#"{"api_key":"sk-live","nested":["ab"]}"#);
}

#[test]
fn safe_json_text_strips_nul_from_keys() {
    let stored = text(json!({"cl\u{0}ient": "value"}), 100);
    assert_eq!(stored, r#"{"client":"value"}"#);
}

#[test]
fn safe_json_text_writes_compact_text() {
    let stored = text(json!({"b": 1, "a": [true, null]}), 100);
    assert_eq!(stored, r#"{"a":[true,null],"b":1}"#);
}

#[test]
fn safe_json_text_keeps_a_value_at_the_cap() {
    let stored = text(json!({"outer": 5}), 1);
    assert_eq!(stored, r#"{"outer":5}"#);
    assert_eq!(text(json!(5), 0), "5");
}

#[rstest]
#[case::root(json!({"outer": {"inner": 1}}), 0, r#"{"outer":"MaxDepthExceeded"}"#)]
#[case::nested(json!({"outer": {"inner": {"deep": 1}}}), 1, r#"{"outer":{"inner":"MaxDepthExceeded"}}"#)]
#[case::array(json!([[[1]]]), 1, r#"[[["MaxDepthExceeded"]]]"#)]
fn safe_json_text_replaces_a_value_past_the_cap(
    #[case] value: AgentJson,
    #[case] max_depth: usize,
    #[case] expected: &str,
) {
    assert_eq!(text(value, max_depth), expected);
}

#[test]
fn safe_json_text_keeps_scalars_and_arrays() {
    let stored = text(
        json!({"count": 3, "flag": false, "empty": null, "list": []}),
        100,
    );
    assert_eq!(stored, r#"{"count":3,"empty":null,"flag":false,"list":[]}"#);
}
