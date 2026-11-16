#![cfg(test)]

use rstest::rstest;

use super::string_choice_contents;
use crate::guardrails::chat_guardrail_translation::rewrites::tests::support::response;

#[rstest]
#[case::one_string(r#"{"choices": [{"message": {"content": "hi"}}]}"#, &[Some("hi")])]
#[case::empty_string(r#"{"choices": [{"message": {"content": ""}}]}"#, &[Some("")])]
#[case::list_content(r#"{"choices": [{"message": {"content": [{"text": "hi"}]}}]}"#, &[None])]
#[case::null_content(r#"{"choices": [{"message": {"content": null}}]}"#, &[None])]
#[case::tool_call_only(r#"{"choices": [{"message": {"tool_calls": []}}]}"#, &[None])]
#[case::one_entry_per_choice_in_order(
    r#"{"choices": [{"message": {"content": "a"}}, {"message": {"content": null}}, {"message": {"content": "c"}}]}"#,
    &[Some("a"), None, Some("c")]
)]
#[case::no_choices(r#"{"choices": []}"#, &[])]
fn string_choice_contents_keeps_one_entry_per_choice(
    #[case] source: &str,
    #[case] expected: &[Option<&str>],
) {
    let expected: Vec<Option<String>> = expected
        .iter()
        .map(|content| content.map(str::to_owned))
        .collect();
    assert_eq!(string_choice_contents(&response(source)), expected);
}
