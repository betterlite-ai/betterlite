#![cfg(test)]

use rstest::rstest;

use super::{grounding_block_message, scan_block_message};
use crate::xecguard_primitives::wire::ScanResult;

fn scan_result(wire: &str) -> ScanResult {
    serde_json::from_str(wire).expect("scan result")
}

#[test]
fn block_message_lists_unique_policies_and_the_trace_id() {
    let message = scan_block_message(&scan_result(
        r#"{"trace_id":"trace-1","xecguard_result":[{"violated_policy_name":"A"},{"violated_policy_name":"A"},{"violated_policy_name":"B"},{"violated_policy_name":""},{"violated_policy_name":3}]}"#,
    ));
    assert_eq!(
        message,
        "Blocked by XecGuard: policies=[A,B] trace_id=trace-1 rationale="
    );
}

#[rstest]
#[case::missing("{}")]
#[case::null_result(r#"{"xecguard_result":null}"#)]
#[case::object_result(r#"{"xecguard_result":{"violated_policy_name":"A"}}"#)]
#[case::empty_list(r#"{"xecguard_result":[]}"#)]
fn block_message_defaults_policies_to_unknown(#[case] wire: &str) {
    assert_eq!(
        scan_block_message(&scan_result(wire)),
        "Blocked by XecGuard: policies=[unknown] trace_id= rationale="
    );
}

#[test]
fn block_message_uses_the_first_non_empty_rationale() {
    let message = scan_block_message(&scan_result(
        r#"{"trace_id":"t","xecguard_result":[{"rationale":""},{"rationale":"because"},{"rationale":"later"}]}"#,
    ));
    assert_eq!(
        message,
        "Blocked by XecGuard: policies=[unknown] trace_id=t rationale=because"
    );
}

#[test]
fn block_message_truncates_the_rationale_to_two_hundred_characters() {
    let wire = format!(
        r#"{{"xecguard_result":[{{"rationale":"{}"}}]}}"#,
        "a".repeat(250)
    );
    let message = scan_block_message(&scan_result(&wire));
    assert!(message.ends_with(&format!("rationale={}", "a".repeat(200))));
}

#[test]
fn block_message_truncates_characters_not_bytes() {
    let wire = format!(
        r#"{{"xecguard_result":[{{"rationale":"{}"}}]}}"#,
        "\u{e9}".repeat(250)
    );
    let message = scan_block_message(&scan_result(&wire));
    assert!(message.ends_with(&format!("rationale={}", "\u{e9}".repeat(200))));
}

#[test]
fn grounding_block_message_lists_only_string_rules() {
    let message = grounding_block_message(&scan_result(
        r#"{"trace_id":"g-1","xecguard_result":{"violated_rules_list":["R1",3,"R2",null],"rationale":"why"}}"#,
    ));
    assert_eq!(
        message,
        "Blocked by XecGuard grounding: rules=[R1,R2] trace_id=g-1 rationale=why"
    );
}

#[rstest]
#[case::missing("{}")]
#[case::null_result(r#"{"xecguard_result":null}"#)]
#[case::list_result(r#"{"xecguard_result":[]}"#)]
#[case::empty_rules(r#"{"xecguard_result":{"violated_rules_list":[]}}"#)]
fn grounding_block_message_defaults_rules_to_unknown(#[case] wire: &str) {
    assert!(
        grounding_block_message(&scan_result(wire))
            .starts_with("Blocked by XecGuard grounding: rules=[unknown] trace_id= rationale=")
    );
}

#[test]
fn grounding_block_message_truncates_the_rationale() {
    let wire = format!(
        r#"{{"xecguard_result":{{"rationale":"{}"}}}}"#,
        "b".repeat(250)
    );
    let message = grounding_block_message(&scan_result(&wire));
    assert!(message.ends_with(&format!("rationale={}", "b".repeat(200))));
}

#[test]
fn grounding_block_message_keeps_an_empty_rationale() {
    let message = grounding_block_message(&scan_result(
        r#"{"xecguard_result":{"violated_rules_list":["R1"],"rationale":""}}"#,
    ));
    assert_eq!(
        message,
        "Blocked by XecGuard grounding: rules=[R1] trace_id= rationale="
    );
}
