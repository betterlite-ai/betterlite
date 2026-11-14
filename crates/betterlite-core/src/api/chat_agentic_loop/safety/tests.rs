#![cfg(test)]

use betterlite_context::agentic_loop_plan::JsonMap;

use super::{SafetyError, check_loop_safety};
use crate::api::chat_agentic_loop::tests::support::json_map;

fn tools() -> JsonMap {
    json_map(&[
        ("b", r#"{"name":"b"}"#),
        ("a", "[1,2]"),
        ("listed", r#"[{"b":1,"a":2}]"#),
        ("nested", r#"{"z":{"y":1,"x":2},"a":3}"#),
    ])
}

#[test]
fn the_fingerprint_is_sorted_key_json() {
    let fingerprint = check_loop_safety(&tools(), &[], 0, 3, "gpt-4o").unwrap();
    assert_eq!(
        fingerprint,
        r#"{"a":[1,2],"b":{"name":"b"},"listed":[{"a":2,"b":1}],"nested":{"a":3,"z":{"x":2,"y":1}}}"#
    );
}

#[test]
fn a_repeated_fingerprint_is_refused_before_the_depth_cap() {
    let fingerprint = check_loop_safety(&tools(), &[], 0, 3, "gpt-4o").unwrap();
    let error = check_loop_safety(&tools(), &[fingerprint], 5, 3, "gpt-4o").unwrap_err();
    assert_eq!(error, SafetyError::RepeatedFingerprint);
    assert_eq!(
        error.to_string(),
        "Agentic loop detected repeated tool-call fingerprint; aborting rerun"
    );
}

#[test]
fn a_depth_at_the_cap_is_exceeded() {
    let error = check_loop_safety(&json_map(&[("call", "1")]), &[], 3, 3, "gpt-4o").unwrap_err();
    assert_eq!(
        error,
        SafetyError::MaxLoopsExceeded {
            max_loops: 3,
            model: "gpt-4o".to_owned(),
        }
    );
    assert_eq!(
        error.to_string(),
        "Exceeded max_agentic_loops=3 for model=gpt-4o"
    );
}

#[test]
fn a_depth_below_the_cap_passes() {
    assert!(check_loop_safety(&json_map(&[("call", "1")]), &[], 2, 3, "gpt-4o").is_ok());
}

#[test]
fn the_fingerprint_text_escapes_control_characters() {
    let calls = json_map(&[("call", r#""a\"b\\c\n\r\t\b\f\u0001 d""#)]);
    let fingerprint = check_loop_safety(&calls, &[], 0, 3, "gpt-4o").unwrap();
    assert_eq!(fingerprint, r#"{"call":"a\"b\\c\n\r\t\b\f\u0001 d"}"#);
}
