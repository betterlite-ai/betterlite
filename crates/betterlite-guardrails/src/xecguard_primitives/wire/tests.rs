#![cfg(test)]

use rstest::rstest;

use super::{ScanResult, ScanType};

fn scan_result(wire: &str) -> ScanResult {
    serde_json::from_str(wire).expect("scan result")
}

#[rstest]
#[case::unsafe_decision(r#"{"decision":"UNSAFE"}"#, true)]
#[case::lowercase_decision(r#"{"decision":"unsafe"}"#, false)]
#[case::non_text_decision(r#"{"decision":true}"#, false)]
#[case::null_decision(r#"{"decision":null}"#, false)]
#[case::missing_decision(r#"{"trace_id":"t"}"#, false)]
fn scan_result_flags_only_the_unsafe_text_decision(
    #[case] wire: &str,
    #[case] unsafe_decision: bool,
) {
    assert_eq!(scan_result(wire).is_unsafe(), unsafe_decision);
}

#[test]
fn scan_result_keeps_unmodelled_fields_and_omits_absent_ones() {
    let parsed =
        scan_result(r#"{"decision":"SAFE","trace_id":"t-1","xecguard_result":[],"extra":{"a":1}}"#);
    assert_eq!(parsed.trace_id_text(), "t-1");
    assert!(parsed.extra.contains_key("extra"));
    let serialized = serde_json::to_string(&parsed).expect("serialize");
    assert!(serialized.contains("\"extra\":{\"a\":1}"));
    let empty = scan_result("{}");
    assert!(
        !serde_json::to_string(&empty)
            .expect("serialize")
            .contains("decision")
    );
}

#[test]
fn scan_result_fields_rebuilds_the_whole_object() {
    let parsed =
        scan_result(r#"{"decision":"UNSAFE","trace_id":"t","xecguard_result":[],"other":1}"#);
    let fields = parsed.fields();
    assert_eq!(fields.len(), 4);
    assert!(fields.contains_key("decision"));
    assert!(fields.contains_key("trace_id"));
    assert!(fields.contains_key("xecguard_result"));
    assert!(fields.contains_key("other"));
}

#[test]
fn scan_type_serializes_lowercase() {
    assert_eq!(
        serde_json::to_string(&ScanType::Response).expect("serialize"),
        "\"response\""
    );
    assert_eq!(
        serde_json::to_string(&ScanType::Input).expect("serialize"),
        "\"input\""
    );
}
