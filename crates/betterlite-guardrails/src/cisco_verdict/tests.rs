#![cfg(test)]

use rstest::rstest;

use crate::cisco_conversation::json::Json;
use crate::cisco_verdict::Action;

#[rstest]
#[case::null("null", "{}")]
#[case::array("[]", "{}")]
#[case::no_decision(r#"{"other":1}"#, r#"{"other":1}"#)]
fn cisco_verdict_unwrap_fallback(#[case] input: &str, #[case] expected: &str) {
    let Json::Object(fields) = super::unwrap_verdict_envelope(&Json::parse(input)) else {
        panic!("unwrapping must return an object");
    };
    assert_eq!(serde_json::to_string(&fields).unwrap(), expected);
}

#[test]
fn cisco_verdict_unwraps_nested_result() {
    let value = super::unwrap_verdict_envelope(&Json::parse(r#"{"result": {"action": "allow"}}"#));
    assert!(super::decision::has_decision_fields(&value));
}

#[test]
fn cisco_verdict_ignores_empty_nested_match() {
    let value = super::unwrap_verdict_envelope(&Json::parse(
        r#"{"result": {}, "data": {"action": "allow"}}"#,
    ));
    assert_eq!(
        super::normalize_action(&value_action(&value)),
        Action::Allow
    );
}

fn value_action(value: &Json) -> String {
    let Json::Object(fields) = value else {
        return String::new();
    };
    match Json::field(fields, "action") {
        Json::Text(action) => action,
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
            String::new()
        }
    }
}

#[test]
fn cisco_verdict_normalizes_actions() {
    assert_eq!(super::normalize_action(" DENIED "), Action::Block);
    assert_eq!(super::normalize_action("sanitized"), Action::Redact);
    assert_eq!(super::normalize_action("SAFE"), Action::Allow);
}

#[test]
fn cisco_verdict_ignores_data_nested_sanitized_text() {
    let value = Json::parse(r#"{"data": {"sanitizedText": "clean", "error": {"code": 1}}}"#);
    assert_eq!(super::extract_sanitized_text(&value), None);
    assert!(super::extract_jsonrpc_error(&value).is_none());
    let error = Json::parse(r#"{"result": {"error": {"code": 2}}}"#);
    assert!(super::extract_jsonrpc_error(&error).is_some());
}

#[test]
fn cisco_verdict_extracts_sanitized_text_from_result() {
    let value = Json::parse(r#"{"result": {"sanitizedText": "clean"}}"#);
    assert_eq!(
        super::extract_sanitized_text(&value).as_deref(),
        Some("clean")
    );
}

#[test]
fn cisco_verdict_ignores_direct_arguments_outside_payload() {
    let value = Json::parse(r#"{"arguments": {"q": "x"}}"#);
    assert!(super::extract_mcp_args(&value).is_none());
}

#[test]
fn cisco_verdict_counts_only_string_entities() {
    let rules = Json::parse(r#"[{"entity_types": ["PII", 3, null]}, {"entity_types": ["PII"]}]"#);
    let counts = super::extract_masked_entity_count(&rules).expect("counts");
    assert_eq!(counts.get("PII"), Some(&2));
    assert_eq!(counts.len(), 1);
}
