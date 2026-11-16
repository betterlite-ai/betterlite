#![cfg(test)]

use rstest::rstest;

use super::apply_output_tool_calls;
use crate::guardrails::chat_guardrail_translation::extraction::apply_error::ApplyError;
use crate::guardrails::chat_guardrail_translation::rewrites::error::RewriteError;
use crate::guardrails::chat_guardrail_translation::rewrites::tests::support::{object, response};

const TWO_CALLS: &str = r#"{"choices": [{"message": {"tool_calls": [
    {"id": "1", "type": "function", "function": {"name": "f", "arguments": "{}"}},
    {"id": "2", "type": "function", "function": {"name": "g", "arguments": "{}"}}]}}]}"#;

#[rstest]
#[case::name_and_arguments(
    r#"{"function": {"name": "n", "arguments": "{\"a\":1}"}}"#,
    r#"{"name": "n", "arguments": "{\"a\":1}"}"#
)]
#[case::name_only(
    r#"{"function": {"name": "n"}}"#,
    r#"{"name": "n", "arguments": "{}"}"#
)]
#[case::arguments_only(
    r#"{"function": {"arguments": "x"}}"#,
    r#"{"name": "f", "arguments": "x"}"#
)]
#[case::null_name(
    r#"{"function": {"name": null}}"#,
    r#"{"name": null, "arguments": "{}"}"#
)]
fn apply_output_tool_updates_the_function_name_and_arguments(
    #[case] guardrailed: &str,
    #[case] function: &str,
) {
    let mut subject = response(TWO_CALLS);
    apply_output_tool_calls(&mut subject, &[object(guardrailed)], &[(0, 0)]).unwrap();
    let expected = TWO_CALLS.replacen(r#"{"name": "f", "arguments": "{}"}"#, function, 1);
    assert_eq!(subject, response(&expected));
}

#[test]
fn apply_output_tool_keeps_the_id_and_type_of_the_response_call() {
    let mut subject = response(TWO_CALLS);
    let guardrailed = object(r#"{"id": "9", "type": "other", "function": {"name": "n"}}"#);
    apply_output_tool_calls(&mut subject, &[guardrailed], &[(0, 1)]).unwrap();
    let expected = TWO_CALLS.replace(r#""name": "g""#, r#""name": "n""#);
    assert_eq!(subject, response(&expected));
}

#[test]
fn apply_output_tool_matches_each_call_by_its_mapping() {
    let mut subject = response(TWO_CALLS);
    let first = object(r#"{"function": {"name": "one"}}"#);
    let second = object(r#"{"function": {"name": "two"}}"#);
    apply_output_tool_calls(&mut subject, &[first, second], &[(0, 1), (0, 0)]).unwrap();
    let expected = TWO_CALLS
        .replace(r#""name": "g""#, r#""name": "one""#)
        .replace(r#""name": "f""#, r#""name": "two""#);
    assert_eq!(subject, response(&expected));
}

#[test]
fn apply_output_tool_skips_calls_the_guardrail_did_not_return() {
    let mut subject = response(TWO_CALLS);
    let only = object(r#"{"function": {"name": "n"}}"#);
    apply_output_tool_calls(&mut subject, &[only], &[(0, 0), (0, 1)]).unwrap();
    let expected = TWO_CALLS.replace(r#""name": "f""#, r#""name": "n""#);
    assert_eq!(subject, response(&expected));
}

#[rstest]
#[case::no_function(r#"{"id": "x"}"#)]
#[case::function_that_is_not_an_object(r#"{"function": "g"}"#)]
fn apply_output_tool_ignores_a_guardrailed_call_without_function_fields(#[case] guardrailed: &str) {
    let mut subject = response(TWO_CALLS);
    apply_output_tool_calls(&mut subject, &[object(guardrailed)], &[(0, 0)]).unwrap();
    assert_eq!(subject, response(TWO_CALLS));
}

#[rstest]
#[case::call_index_past_the_end((0, 7))]
#[case::choice_without_tool_calls((1, 0))]
fn apply_output_tool_skips_a_call_the_response_does_not_have(#[case] mapping: (usize, usize)) {
    let source = r#"{"choices": [{"message": {"tool_calls": [{"id": "1", "function": {"name": "f"}}]}}, {"message": {"content": "x"}}]}"#;
    let mut subject = response(source);
    let guardrailed = object(r#"{"function": {"name": "n"}}"#);
    apply_output_tool_calls(&mut subject, &[guardrailed], &[mapping]).unwrap();
    assert_eq!(subject, response(source));
}

#[test]
fn apply_output_tool_ignores_a_call_that_has_no_function_in_the_response() {
    let source = r#"{"choices": [{"message": {"tool_calls": [{"id": "1"}]}}]}"#;
    let mut subject = response(source);
    let guardrailed = object(r#"{"function": {"name": "n"}}"#);
    apply_output_tool_calls(&mut subject, &[guardrailed], &[(0, 0)]).unwrap();
    assert_eq!(subject, response(source));
}

#[test]
fn apply_output_tool_refuses_a_choice_the_response_does_not_have() {
    let mut subject = response(TWO_CALLS);
    let guardrailed = object(r#"{"function": {"name": "n"}}"#);
    let failure = apply_output_tool_calls(&mut subject, &[guardrailed], &[(3, 0)]).unwrap_err();
    assert!(matches!(
        failure,
        RewriteError::Apply(ApplyError::MissingEntry { index: 3 })
    ));
}

#[rstest]
#[case::name(r#"{"function": {"name": 5}}"#, "name")]
#[case::arguments(r#"{"function": {"arguments": {"a": 1}}}"#, "arguments")]
fn apply_output_tool_refuses_function_fields_that_are_not_text(
    #[case] guardrailed: &str,
    #[case] field: &str,
) {
    let mut subject = response(TWO_CALLS);
    let failure =
        apply_output_tool_calls(&mut subject, &[object(guardrailed)], &[(0, 0)]).unwrap_err();
    assert!(matches!(failure, RewriteError::FieldNotText { field: named } if named == field));
}
