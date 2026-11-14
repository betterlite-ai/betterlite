#![cfg(test)]

use betterlite_context::agentic_loop_plan::JsonMap;
use betterlite_models::decisions::Json;
use serde_json::Number;

use super::{SafetyError, check_loop_safety, json_text};
use crate::api::chat_agentic_loop::tests::support::json_map;

fn tools() -> JsonMap {
    json_map(&[("b", r#"{"name":"b"}"#), ("a", "[1,2]")])
}

#[test]
fn the_fingerprint_is_sorted_key_json() {
    let fingerprint = check_loop_safety(&tools(), &[], 0, 3, "gpt-4o").unwrap();
    assert_eq!(fingerprint, r#"{"a":[1,2],"b":{"name":"b"}}"#);
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
fn json_text_escapes_the_control_characters() {
    let text = Json::Text("a\"b\\c\n\r\t\u{08}\u{0c}\u{1} d".to_owned());
    assert_eq!(json_text(&text), r#""a\"b\\c\n\r\t\b\f\u0001 d""#);
}

#[test]
fn json_text_renders_every_shape() {
    assert_eq!(json_text(&Json::Null), "null");
    assert_eq!(json_text(&Json::Bool(true)), "true");
    assert_eq!(json_text(&Json::Bool(false)), "false");
    assert_eq!(json_text(&Json::Number(Number::from(7))), "7");
    assert_eq!(
        json_text(&Json::Number(Number::from_f64(1.5).unwrap())),
        "1.5"
    );
    assert_eq!(
        json_text(&Json::List(vec![Json::Bool(false), Json::Null])),
        "[false,null]"
    );
    let object = Json::Object(
        [
            ("z".to_owned(), Json::Null),
            ("a".to_owned(), Json::Bool(true)),
        ]
        .into_iter()
        .collect(),
    );
    assert_eq!(json_text(&object), r#"{"a":true,"z":null}"#);
}
