#![cfg(test)]

use super::{Input, Op};
use crate::helpers::Helper as _;
use rstest::rstest;

#[rstest]
#[case::raise_exception(
    r#"{"mode":"render","chat_template":"{{ raise_exception('boom') }}","messages":[{"role":"user","content":"hi"}]}"#,
    "Error message - boom"
)]
#[case::plain_token(r#"{"mode":"extract_token","token":"tok"}"#, "tok")]
#[case::object_token(r#"{"mode":"extract_token","token":{"content":"c"}}"#, "c")]
#[case::absent_token(r#"{"mode":"extract_token","token":null}"#, "")]
fn jinja_prompts_behavior_tests_behavior(#[case] wire: &str, #[case] expected: &str) {
    let input: Input = serde_json::from_str(wire).unwrap();
    assert_eq!(Op::run(&input).unwrap(), serde_json::to_value(expected).unwrap());
}

#[test]
fn render_errors_keep_the_litellm_class() {
    let input: Input = serde_json::from_str(
        r#"{"mode":"render","chat_template":"{%- if add_generation_prompt is defined and add_generation_prompt -%}{{ undefined_helper() }}{%- endif -%}ok","messages":[{"role":"user","content":"hi"}]}"#,
    )
    .unwrap();
    let error = Op::run(&input).unwrap_err();
    assert_eq!(error.class, "Exception");
    assert_eq!(error.status, None);
}
