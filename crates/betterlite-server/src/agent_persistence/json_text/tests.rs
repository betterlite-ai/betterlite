#![cfg(test)]

use betterlite_core::agents::json::AgentJson;
use rstest::rstest;

use super::safe_json_text;

fn value(source: &str) -> AgentJson {
    serde_json::from_str(source).expect("a valid agent JSON")
}

fn text(value: &AgentJson, max_depth: usize) -> String {
    safe_json_text(value, max_depth)
}

#[test]
fn safe_json_text_strips_nul_from_values() {
    let stored = text(
        &value(r#"{"api_key": "sk-\u0000live", "nested": ["a\u0000b"]}"#),
        100,
    );
    assert_eq!(stored, r#"{"api_key":"sk-live","nested":["ab"]}"#);
}

#[test]
fn safe_json_text_strips_nul_from_keys() {
    let stored = text(&value(r#"{"cl\u0000ient": "value"}"#), 100);
    assert_eq!(stored, r#"{"client":"value"}"#);
}

#[test]
fn safe_json_text_writes_compact_text() {
    let stored = text(&value(r#"{"b": 1, "a": [true, null]}"#), 100);
    assert_eq!(stored, r#"{"a":[true,null],"b":1}"#);
}

#[test]
fn safe_json_text_keeps_a_value_at_the_cap() {
    let stored = text(&value(r#"{"outer": 5}"#), 1);
    assert_eq!(stored, r#"{"outer":5}"#);
    assert_eq!(text(&value("5"), 0), "5");
}

#[rstest]
#[case::root(r#"{"outer": {"inner": 1}}"#, 0, r#"{"outer":"MaxDepthExceeded"}"#)]
#[case::nested(
    r#"{"outer": {"inner": {"deep": 1}}}"#,
    1,
    r#"{"outer":{"inner":"MaxDepthExceeded"}}"#
)]
#[case::array(r#"[[[1]]]"#, 1, r#"[[["MaxDepthExceeded"]]]"#)]
fn safe_json_text_replaces_a_value_past_the_cap(
    #[case] source: &str,
    #[case] max_depth: usize,
    #[case] expected: &str,
) {
    assert_eq!(text(&value(source), max_depth), expected);
}

#[test]
fn safe_json_text_keeps_scalars_and_arrays() {
    let stored = text(
        &value(r#"{"count": 3, "flag": false, "empty": null, "list": []}"#),
        100,
    );
    assert_eq!(stored, r#"{"count":3,"empty":null,"flag":false,"list":[]}"#);
}
