#![cfg(test)]

use rstest::rstest;

use super::first_message_text;
use crate::straiker::v3::fixtures::object;

#[rstest]
#[case::skips_system_message(r#"{"messages":[{"role":"system","content":"rules"},{"role":"user","content":"first"},{"role":"user","content":"second"}]}"#, "first")]
#[case::skips_developer_and_assistant(r#"{"messages":[{"role":"developer","content":"rules"},{"role":"assistant","content":"hello"},{"role":"user","content":"q"}]}"#, "q")]
#[case::joins_text_blocks(r#"{"messages":[{"role":"user","content":[{"type":"image"},{"type":"text","text":"look"},{"type":"text","text":"here"}]}]}"#, "look\nhere")]
#[case::user_without_text_does_not_fall_through(
    r#"{"messages":[{"role":"user"}],"input":"plain","prompt":"p"}"#,
    ""
)]
#[case::responses_input_messages(r#"{"input":[{"role":"user","content":"asked"}]}"#, "asked")]
#[case::input_string(r#"{"input":"plain","prompt":"p"}"#, "plain")]
#[case::prompt_when_input_is_not_text(r#"{"input":3,"prompt":"p"}"#, "p")]
#[case::prompt_string(r#"{"prompt":"complete me"}"#, "complete me")]
#[case::prompt_list_has_no_text(r#"{"prompt":["a","b"]}"#, "")]
#[case::system_only(r#"{"messages":[{"role":"system","content":"rules"}]}"#, "")]
#[case::empty("{}", "")]
fn finds_first_user_text(#[case] source: &str, #[case] expected: &str) {
    assert_eq!(first_message_text(&object(source)), expected);
}
