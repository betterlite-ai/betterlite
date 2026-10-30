#![cfg(test)]

use crate::cisco_conversation::json::Json;

#[test]
fn cisco_verdict_unwraps_nested_result() {
    let value = super::unwrap_verdict_envelope(&Json::parse(r#"{"result":{"action":"allow"}}"#));
    assert!(super::has_decision_fields(&value));
}

#[test]
fn cisco_verdict_normalizes_actions() {
    assert_eq!(super::normalize_action(" DENIED "), "block");
    assert_eq!(super::normalize_action("sanitized"), "redact");
    assert_eq!(super::normalize_action("SAFE"), "allow");
}

#[test]
fn cisco_verdict_extracts_rewrites_and_errors() {
    let value = Json::parse(r#"{"data":{"sanitizedText":"clean","error":{"code":1}}}"#);
    assert_eq!(
        super::extract_sanitized_text(&value).as_deref(),
        Some("clean")
    );
    assert!(super::extract_jsonrpc_error(&value).is_none());
    let error = Json::parse(r#"{"result":{"error":{"code":2}}}"#);
    assert!(super::extract_jsonrpc_error(&error).is_some());
}
