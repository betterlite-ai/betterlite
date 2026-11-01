#![cfg(test)]

use rstest::rstest;

use crate::cisco_conversation::json::Json;
use crate::cisco_mcp::normalize::{self, ResultValue};

fn normalize(input: &str) -> normalize::NormalizedResponse {
    normalize::normalize_response(&Json::parse(input)).expect("normalizable response")
}

fn result_json(result: &ResultValue) -> String {
    serde_json::to_string(result).unwrap()
}

#[test]
fn normalize_passes_jsonrpc_shaped_response_through() {
    let response = normalize(r#"{"jsonrpc": "2.0", "id": 7, "result": {"content": []}}"#);
    assert_eq!(
        result_json(response.result.as_ref().unwrap()),
        r#"{"content": []}"#
    );
    assert_eq!(serde_json::to_string(&response.id_override).unwrap(), "7");
    assert!(response.error.is_none());
}

#[test]
fn normalize_passes_jsonrpc_error_through() {
    let response =
        normalize(r#"{"jsonrpc": "2.0", "id": 1, "error": {"code": -1, "message": "no"}}"#);
    assert!(response.result.is_none());
    assert_eq!(
        serde_json::to_string(&response.error).unwrap(),
        r#"{"code": -1, "message": "no"}"#
    );
}

#[rstest]
#[case::null_id(r#"{"jsonrpc": "2.0", "id": null, "result": {}}"#)]
#[case::default_marker(r#"{"jsonrpc": "2.0", "id": "litellm-mcp", "result": {}}"#)]
fn normalize_jsonrpc_id_override_skips_null_and_default_marker(#[case] input: &str) {
    assert!(normalize(input).id_override.is_none());
}

#[test]
fn normalize_builds_result_from_dict_result_field() {
    let response =
        normalize(r#"{"id": 9, "result": {"content": [{"type": "text", "text": "hi"}]}}"#);
    assert_eq!(
        result_json(response.result.as_ref().unwrap()),
        r#"{"content": [{"type": "text", "text": "hi"}]}"#
    );
    assert_eq!(serde_json::to_string(&response.id_override).unwrap(), "9");
}

#[rstest]
#[case::null_id(r#"{"id": null, "result": {"content": []}}"#)]
#[case::falsy_id(r#"{"id": 0, "result": {"content": []}}"#)]
#[case::default_marker(r#"{"id": "litellm-mcp", "result": {"content": []}}"#)]
fn normalize_result_dict_id_override_requires_truthy_non_default_id(#[case] input: &str) {
    assert!(normalize(input).id_override.is_none());
}

#[test]
fn normalize_builds_result_from_content_list_and_carries_mirror_fields() {
    let response = normalize(
        r#"{"content": [{"type": "text", "text": "hi"}, "raw"], "structuredContent": {"a": 1}, "isError": true}"#,
    );
    let ResultValue::Built(result) = response.result.as_ref().unwrap() else {
        panic!("expected a built result");
    };
    assert_eq!(result.content.len(), 2);
    assert_eq!(
        serde_json::to_string(&result.content[0]).unwrap(),
        r#"{"type": "text", "text": "hi"}"#
    );
    assert_eq!(
        serde_json::to_string(&result.content[1]).unwrap(),
        r#"{"type":"text","text":"raw"}"#
    );
    assert_eq!(
        serde_json::to_string(&result.structured_content).unwrap(),
        r#"{"a": 1}"#
    );
    assert_eq!(result.is_error, Some(true));
}

#[test]
fn normalize_content_list_falls_back_to_snake_case_mirror_fields() {
    let response =
        normalize(r#"{"content": [], "structured_content": {"b": 2}, "is_error": false}"#);
    let ResultValue::Built(result) = response.result.as_ref().unwrap() else {
        panic!("expected a built result");
    };
    assert_eq!(
        serde_json::to_string(&result.structured_content).unwrap(),
        r#"{"b": 2}"#
    );
    assert_eq!(result.is_error, Some(false));
}

#[test]
fn normalize_builds_result_from_pair_list_with_nested_content() {
    let response =
        normalize(r#"[["content", [{"type": "text", "text": "hi"}]], ["isError", false]]"#);
    let ResultValue::Built(result) = response.result.as_ref().unwrap() else {
        panic!("expected a built result");
    };
    assert_eq!(result.content.len(), 1);
    assert_eq!(result.is_error, Some(false));
}

#[test]
fn normalize_pair_list_without_content_key_yields_no_envelope() {
    assert!(normalize::normalize_response(&Json::parse(r#"[["other", 1]]"#)).is_none());
}

#[test]
fn normalize_builds_result_from_bare_content_list() {
    let response = normalize(r#"[{"type": "text", "text": "a"}, {"type": "text", "text": "b"}]"#);
    let ResultValue::Built(result) = response.result.as_ref().unwrap() else {
        panic!("expected a built result");
    };
    assert_eq!(result.content.len(), 2);
    assert!(result.structured_content.is_none());
    assert!(result.is_error.is_none());
}

#[rstest]
#[case::null("null")]
#[case::bool("true")]
#[case::number("1")]
#[case::text(r#""plain""#)]
#[case::empty_object("{}")]
fn normalize_rejects_unshaped_responses(#[case] input: &str) {
    assert!(normalize::normalize_response(&Json::parse(input)).is_none());
}
