#![cfg(test)]

use rstest::rstest;

use super::{KeyRequest, ResetSpendRequest};
use crate::proxy_request_contracts::tests::{
    assert_decode, assert_validation, contract, decode_text,
};

#[rstest]
#[case::empty("{}")]
#[case::keys_null(r#"{"keys": null}"#)]
#[case::aliases_null(r#"{"key_aliases": null}"#)]
#[case::both_empty(r#"{"keys": [], "key_aliases": []}"#)]
#[case::keys_empty_aliases_null(r#"{"keys": [], "key_aliases": null}"#)]
#[case::keys_null_aliases_empty(r#"{"keys": null, "key_aliases": []}"#)]
fn rejects_a_body_without_a_key_or_alias(#[case] text: &str) {
    let error = KeyRequest::from_value(&contract(text)).expect_err("the request names nothing");
    assert_validation(
        error,
        "At least one of 'keys' or 'key_aliases' must be provided.",
    );
}

#[rstest]
#[case::keys(r#"{"keys": ["sk-1"]}"#, r#"{"keys":["sk-1"],"key_aliases":null}"#)]
#[case::aliases(r#"{"key_aliases": ["a"]}"#, r#"{"keys":null,"key_aliases":["a"]}"#)]
#[case::both(
    r#"{"keys": ["sk-1"], "key_aliases": ["a"]}"#,
    r#"{"keys":["sk-1"],"key_aliases":["a"]}"#
)]
#[case::aliases_with_empty_keys(
    r#"{"keys": [], "key_aliases": ["a"]}"#,
    r#"{"keys":[],"key_aliases":["a"]}"#
)]
fn accepts_a_bound_list(#[case] text: &str, #[case] dumped: &str) {
    let request = KeyRequest::from_value(&contract(text)).expect("a list of keys or aliases");
    assert_eq!(serde_json::to_string(&request).expect("the dump"), dumped);
}

#[rstest]
#[case::text(r#"{"keys": "sk-1"}"#)]
#[case::numbers(r#"{"keys": [1, 2]}"#)]
#[case::object(r#"{"key_aliases": {"a": 1}}"#)]
fn rejects_a_non_list(#[case] text: &str) {
    let error = KeyRequest::from_value(&contract(text)).expect_err("a list of strings");
    assert_decode(error);
}

// The decode error names the field and the element the caller has to fix.
#[test]
fn names_the_field_a_decode_error_came_from() {
    let error = KeyRequest::from_value(&contract(r#"{"keys": ["sk-1", 2]}"#))
        .expect_err("a number in the key list");
    let message = decode_text(&error);
    assert!(message.starts_with("keys[1]: "), "{message}");
}

#[test]
fn ignores_unknown_keys() {
    let request = KeyRequest::from_value(&contract(r#"{"keys": ["sk-1"], "extra": true}"#))
        .expect("unknown keys are ignored");
    assert_eq!(request.keys, Some(vec!["sk-1".to_owned()]));
}

#[rstest]
#[case::true_value("true")]
#[case::false_value("false")]
fn rejects_a_boolean_reset_to(#[case] value: &str) {
    let text = format!(r#"{{"reset_to": {value}}}"#);
    let error = ResetSpendRequest::from_value(&contract(&text)).expect_err("a boolean target");
    assert_validation(error, "reset_to must be a number, not a boolean");
}

#[rstest]
#[case::integer("2", r#"{"reset_to":2.0}"#)]
#[case::fraction("1.5", r#"{"reset_to":1.5}"#)]
#[case::zero("0", r#"{"reset_to":0.0}"#)]
fn decodes_a_number_reset_to(#[case] value: &str, #[case] dumped: &str) {
    let text = format!(r#"{{"reset_to": {value}}}"#);
    let request = ResetSpendRequest::from_value(&contract(&text)).expect("a number target");
    assert_eq!(serde_json::to_string(&request).expect("the dump"), dumped);
}

#[rstest]
#[case::missing("{}")]
#[case::null(r#"{"reset_to": null}"#)]
#[case::text(r#"{"reset_to": "five"}"#)]
fn rejects_a_reset_to_that_is_no_number(#[case] text: &str) {
    let error = ResetSpendRequest::from_value(&contract(text)).expect_err("no number to reset to");
    assert_decode(error);
}
