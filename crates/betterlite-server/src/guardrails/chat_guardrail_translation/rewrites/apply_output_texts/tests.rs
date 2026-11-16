#![cfg(test)]

use rstest::rstest;

use super::apply_output_texts;
use crate::guardrails::chat_guardrail_translation::extraction::apply_error::ApplyError;
use crate::guardrails::chat_guardrail_translation::rewrites::error::RewriteError;
use crate::guardrails::chat_guardrail_translation::rewrites::tests::support::response;

const STRING_AND_LIST: &str = r#"{"choices": [
    {"message": {"content": "first"}},
    {"message": {"content": [{"type": "text", "text": "a"}, {"type": "text", "text": "b"}]}}]}"#;

fn texts(responses: &[&str]) -> Vec<String> {
    responses.iter().map(|text| (*text).to_owned()).collect()
}

#[rstest]
#[case::string_content(&["X"], &[(0, None)], r#""first""#, r#""X""#)]
#[case::first_list_item(&["X"], &[(1, Some(0))], r#""text": "a""#, r#""text": "X""#)]
#[case::second_list_item(&["X"], &[(1, Some(1))], r#""text": "b""#, r#""text": "X""#)]
fn apply_output_text_writes_a_response_by_choice_and_content_index(
    #[case] responses: &[&str],
    #[case] mappings: &[(usize, Option<usize>)],
    #[case] replaced: &str,
    #[case] replacement: &str,
) {
    let mut subject = response(STRING_AND_LIST);
    apply_output_texts(&mut subject, &texts(responses), mappings).unwrap();
    assert_eq!(
        subject,
        response(&STRING_AND_LIST.replace(replaced, replacement))
    );
}

#[test]
fn apply_output_text_writes_the_responses_in_order_across_choices() {
    let mut subject = response(STRING_AND_LIST);
    let mappings = [(0, None), (1, Some(0))];
    apply_output_texts(&mut subject, &texts(&["X", "Y"]), &mappings).unwrap();
    let expected = STRING_AND_LIST
        .replace(r#""first""#, r#""X""#)
        .replace(r#""text": "a""#, r#""text": "Y""#);
    assert_eq!(subject, response(&expected));
}

#[test]
fn apply_output_text_applies_the_responses_it_has_when_the_guardrail_returns_fewer() {
    let mut subject = response(STRING_AND_LIST);
    let mappings = [(0, None), (1, Some(0))];
    apply_output_texts(&mut subject, &texts(&["X"]), &mappings).unwrap();
    let expected = STRING_AND_LIST.replace(r#""first""#, r#""X""#);
    assert_eq!(subject, response(&expected));
}

#[rstest]
#[case::list_slot_for_string_content(r#"{"choices": [{"message": {"content": "keep"}}]}"#, (0, Some(0)))]
#[case::string_slot_for_list_content(r#"{"choices": [{"message": {"content": [{"text": "keep"}]}}]}"#, (0, None))]
#[case::null_content(r#"{"choices": [{"message": {"content": null}}]}"#, (0, None))]
fn apply_output_text_skips_a_slot_the_content_does_not_have(
    #[case] source: &str,
    #[case] mapping: (usize, Option<usize>),
) {
    let mut subject = response(source);
    apply_output_texts(&mut subject, &texts(&["X"]), &[mapping]).unwrap();
    assert_eq!(subject, response(source));
}

#[test]
fn apply_output_text_adds_the_text_to_an_item_that_had_none() {
    let source = r#"{"choices": [{"message": {"content": [{"type": "image_url"}]}}]}"#;
    let mut subject = response(source);
    apply_output_texts(&mut subject, &texts(&["X"]), &[(0, Some(0))]).unwrap();
    let expected =
        r#"{"choices": [{"message": {"content": [{"type": "image_url", "text": "X"}]}}]}"#;
    assert_eq!(subject, response(expected));
}

#[test]
fn apply_output_text_refuses_more_responses_than_slots() {
    let mut subject = response(STRING_AND_LIST);
    let failure = apply_output_texts(&mut subject, &texts(&["X", "Y"]), &[(0, None)]).unwrap_err();
    assert!(matches!(
        failure,
        RewriteError::Apply(ApplyError::UnmappedResponse { response_index: 1 })
    ));
}

#[test]
fn apply_output_text_refuses_a_choice_the_response_does_not_have() {
    let mut subject = response(STRING_AND_LIST);
    let failure = apply_output_texts(&mut subject, &texts(&["X"]), &[(2, None)]).unwrap_err();
    assert!(matches!(
        failure,
        RewriteError::Apply(ApplyError::MissingEntry { index: 2 })
    ));
}

#[test]
fn apply_output_text_refuses_a_content_item_the_choice_does_not_have() {
    let mut subject = response(STRING_AND_LIST);
    let failure = apply_output_texts(&mut subject, &texts(&["X"]), &[(1, Some(5))]).unwrap_err();
    assert!(matches!(
        failure,
        RewriteError::Apply(ApplyError::MissingContentItem {
            index: 1,
            content_index: 5
        })
    ));
}
