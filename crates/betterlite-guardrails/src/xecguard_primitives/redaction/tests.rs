#![cfg(test)]

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use super::{redact_match_and_regex_keys, sanitize_scan_result};
use crate::xecguard_primitives::wire::ScanResult;

fn scan_result(wire: &str) -> ScanResult {
    serde_json::from_str(wire).expect("scan result")
}

fn value(wire: &str) -> GuardrailValue {
    serde_json::from_str(wire).expect("json value")
}

#[test]
fn redaction_replaces_match_and_regex_values_at_every_depth() {
    let redacted = redact_match_and_regex_keys(&value(
        r#"{"match":"span","nested":{"regex":"pattern","keep":"this"},"items":[{"match":7},{"keep":"that"}]}"#,
    ));
    assert_eq!(
        redacted,
        value(
            r#"{"match":"[REDACTED]","nested":{"regex":"[REDACTED]","keep":"this"},"items":[{"match":"[REDACTED]"},{"keep":"that"}]}"#
        )
    );
}

#[test]
fn redaction_drops_secret_fields_before_masking() {
    let sanitized = sanitize_scan_result(&scan_result(
        r#"{"decision":"UNSAFE","secret_fields":{"api_key":"sk-live-12345678"},"xecguard_result":[]}"#,
    ))
    .expect("sanitized");
    assert!(!sanitized.contains_key("secret_fields"));
    assert_eq!(
        sanitized.get("decision"),
        Some(&GuardrailValue::Text("UNSAFE".to_owned()))
    );
    let serialized = serde_json::to_string(&sanitized).expect("serialize");
    assert!(!serialized.contains("sk-live"));
}

#[test]
fn redaction_masks_credentials_under_sensitive_keys() {
    let sanitized = sanitize_scan_result(&scan_result(
        r#"{"trace_id":"t","detail":{"authorization":"Bearer sk-abcdefghijkl","note":"visible"},"secret_fields":null}"#,
    ))
    .expect("sanitized");
    let detail = match sanitized.get("detail") {
        Some(GuardrailValue::Object(detail)) => detail,
        other => panic!("expected a detail object, got {other:?}"),
    };
    assert_eq!(
        detail.get("authorization"),
        Some(&GuardrailValue::Text(format!("Bear{}ijkl", "*".repeat(14))))
    );
    assert_eq!(
        detail.get("note"),
        Some(&GuardrailValue::Text("visible".to_owned()))
    );
}

#[test]
fn redaction_masks_credentials_inside_arrays() {
    let sanitized = sanitize_scan_result(&scan_result(
        r#"{"evidence":[{"api_key":"sk-abcdefghijkl","text":"fine"}]}"#,
    ))
    .expect("sanitized");
    let serialized = serde_json::to_string(&sanitized).expect("serialize");
    assert!(!serialized.contains("sk-abcdefghijkl"));
    assert!(serialized.contains("\"text\":\"fine\""));
}

#[test]
fn redaction_keeps_other_fields_untouched() {
    let sanitized = sanitize_scan_result(&scan_result(
        r#"{"decision":"SAFE","trace_id":"t-1","xecguard_result":[{"violated_policy_name":"P"}],"other":{"a":1}}"#,
    ))
    .expect("sanitized");
    assert_eq!(
        sanitized.get("trace_id"),
        Some(&GuardrailValue::Text("t-1".to_owned()))
    );
    assert_eq!(sanitized.get("other"), Some(&value(r#"{"a":1}"#)));
    assert_eq!(sanitized.len(), 4);
}
