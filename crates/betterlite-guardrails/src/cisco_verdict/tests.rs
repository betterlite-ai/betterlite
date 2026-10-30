#![cfg(test)]

use serde_json::json;

#[test]
fn cisco_verdict_unwraps_nested_result() {
    let value = super::unwrap_verdict_envelope(&json!({"result": {"action": "allow"}}));
    assert_eq!(value, json!({"action": "allow"}));
}

#[test]
fn cisco_verdict_normalizes_actions() {
    assert_eq!(super::normalize_action(" DENIED "), "block");
    assert_eq!(super::normalize_action("sanitized"), "redact");
    assert_eq!(super::normalize_action("SAFE"), "allow");
}

#[test]
fn cisco_verdict_extracts_rewrites_and_errors() {
    let value = json!({"data": {"sanitizedText": "clean", "error": {"code": 1}}});
    assert_eq!(
        super::extract_sanitized_text(&value).as_deref(),
        Some("clean")
    );
    assert_eq!(super::extract_jsonrpc_error(&value), None);
    let error = json!({"result": {"error": {"code": 2}}});
    assert_eq!(
        super::extract_jsonrpc_error(&error),
        Some(json!({"code": 2}))
    );
}
