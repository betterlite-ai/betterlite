#![cfg(test)]

use super::*;

#[rstest]
#[case::known("this model's maximum context length is 10", true)]
#[case::validation("string_above_max_length", false)]
#[case::validation_with_context_phrase(
    "string_above_max_length this model's maximum context length is 10",
    false
)]
#[case::invalid_user_with_context_phrase(
    "invalid 'user' this model's maximum context length is 10",
    true
)]
#[case::ordinary("bad request", false)]
#[case::current_length_without_while_limit("current length is 500", false)]
#[case::while_limit_without_current_length("while limit is 400", false)]
#[case::current_length_and_while_limit("current length is 500 while limit is 400", true)]
#[case::maximum_input_length_without_tokens("maximum input length is 10", false)]
#[case::tokens_without_maximum_input_length("some tokens here", false)]
#[case::maximum_input_length_with_tokens("maximum input length is 10 tokens", true)]
fn error_facts_behavior_tests_behavior(#[case] message: &str, #[case] expected: bool) {
    assert_eq!(ExceptionCheckers::context_window(message), expected);
}
