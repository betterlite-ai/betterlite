use betterlite_prompts::arguments::json::Json;

use super::json_fixture;
use crate::message_replay::{
    EmptyKind, is_empty_block, is_empty_unsigned_thinking, is_invalid_thinking_error,
};

#[test]
fn invalid_error_matches_invalid_and_valid_string_signature_texts() {
    assert!(is_invalid_thinking_error(
        "messages.2.content.0.thinking.signature.str: Input should be a valid string"
    ));
    assert!(is_invalid_thinking_error(
        "messages.N.content.M: Invalid `signature` in `thinking` block"
    ));
}

#[test]
fn invalid_error_matches_the_missing_thinking_text() {
    assert!(is_invalid_thinking_error(
        "messages.N.content.M.thinking: each thinking block must contain thinking"
    ));
}

#[test]
fn invalid_error_folds_case() {
    assert!(is_invalid_thinking_error(
        "INPUT SHOULD BE A VALID STRING: THINKING.SIGNATURE"
    ));
}

#[test]
fn invalid_error_rejects_empty_unrelated_and_signature_only_texts() {
    assert!(!is_invalid_thinking_error(""));
    assert!(!is_invalid_thinking_error("rate limit exceeded"));
    assert!(!is_invalid_thinking_error("thinking block is fine"));
    assert!(!is_invalid_thinking_error("signature invalid"));
    assert!(!is_invalid_thinking_error("thinking block is invalid"));
    assert!(!is_invalid_thinking_error(
        "thinking.signature: must be a string"
    ));
}

#[test]
fn empty_text_predicate_matches_missing_non_string_and_blank_text() {
    assert!(is_empty_block(
        &json_fixture(r#"{"type":"text","text":""}"#),
        EmptyKind::Text
    ));
    assert!(is_empty_block(
        &json_fixture(r#"{"type":"text","text":"  \n\t"}"#),
        EmptyKind::Text
    ));
    assert!(is_empty_block(
        &json_fixture(r#"{"type":"text"}"#),
        EmptyKind::Text
    ));
    assert!(is_empty_block(
        &json_fixture(r#"{"type":"text","text":5}"#),
        EmptyKind::Text
    ));
    assert!(!is_empty_block(
        &json_fixture(r#"{"type":"text","text":"hi"}"#),
        EmptyKind::Text
    ));
}

#[test]
fn empty_thinking_predicate_ignores_the_signature() {
    assert!(is_empty_block(
        &json_fixture(r#"{"type":"thinking","thinking":"","signature":"sig"}"#),
        EmptyKind::Thinking
    ));
    assert!(!is_empty_block(
        &json_fixture(r#"{"type":"thinking","thinking":"reasoning"}"#),
        EmptyKind::Thinking
    ));
}

#[test]
fn empty_thinking_predicate_rejects_redacted_thinking() {
    assert!(!is_empty_block(
        &json_fixture(r#"{"type":"redacted_thinking","data":"x"}"#),
        EmptyKind::Thinking
    ));
}

#[test]
fn empty_kind_selects_the_block_type_and_field() {
    let text_block = json_fixture(r#"{"type":"text","text":""}"#);
    let thinking_block = json_fixture(r#"{"type":"thinking","thinking":""}"#);
    assert!(!is_empty_block(&text_block, EmptyKind::Thinking));
    assert!(!is_empty_block(&thinking_block, EmptyKind::Text));
    assert!(is_empty_block(&text_block, EmptyKind::Text));
    assert!(is_empty_block(&thinking_block, EmptyKind::Thinking));
}

// Python's str.strip also removes U+001C to U+001F; see docs/deviations/B571.yaml.
#[test]
fn empty_predicates_keep_the_python_only_separators() {
    let text = json_fixture(r#"{"type":"text","text":"\u001c"}"#);
    let thinking = json_fixture(r#"{"type":"thinking","thinking":"\u001f"}"#);
    assert!(!is_empty_block(&text, EmptyKind::Text));
    assert!(!is_empty_block(&thinking, EmptyKind::Thinking));
}

#[test]
fn empty_predicates_reject_non_objects() {
    assert!(!is_empty_block(
        &Json::Text("text".to_owned()),
        EmptyKind::Text
    ));
    assert!(!is_empty_block(&json_fixture("[]"), EmptyKind::Thinking));
    assert!(!is_empty_unsigned_thinking(&Json::Null));
}

#[rstest::rstest]
#[case::empty_string(r#""""#)]
#[case::null("null")]
#[case::false_literal("false")]
#[case::zero("0")]
#[case::empty_list("[]")]
#[case::empty_object("{}")]
fn unsigned_thinking_accepts_falsey_signatures(#[case] signature: &str) {
    let block = json_fixture(&format!(
        r#"{{"type":"thinking","thinking":"","signature":{signature}}}"#
    ));
    assert!(is_empty_unsigned_thinking(&block));
}

#[rstest::rstest]
#[case::string(r#""sig""#)]
#[case::number("1")]
#[case::true_literal("true")]
#[case::list(r#"["x"]"#)]
#[case::object(r#"{"a":1}"#)]
fn unsigned_thinking_rejects_truthy_signatures(#[case] signature: &str) {
    let block = json_fixture(&format!(
        r#"{{"type":"thinking","thinking":"","signature":{signature}}}"#
    ));
    assert!(!is_empty_unsigned_thinking(&block));
}

#[test]
fn unsigned_thinking_handles_a_missing_or_nonempty_thinking_text() {
    assert!(is_empty_unsigned_thinking(&json_fixture(
        r#"{"type":"thinking","thinking":""}"#
    )));
    assert!(!is_empty_unsigned_thinking(&json_fixture(
        r#"{"type":"thinking","thinking":"real"}"#
    )));
}
