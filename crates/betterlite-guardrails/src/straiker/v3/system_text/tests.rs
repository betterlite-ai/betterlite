#![cfg(test)]

use rstest::rstest;

use super::system_text;
use crate::straiker::v3::fixtures::object;

#[rstest]
#[case::anthropic_system_string(
    r#"{"system":"be brief","messages":[{"role":"system","content":"other"}]}"#,
    Some("be brief")
)]
#[case::empty_system_string_is_present(r#"{"system":"","instructions":"later"}"#, Some(""))]
#[case::system_blocks_as_json(
    r#"{"system":[{"type":"text","text":"be brief"}]}"#,
    Some(r#"[{"type":"text","text":"be brief"}]"#)
)]
#[case::null_system_is_absent(r#"{"system":null,"instructions":"steer"}"#, Some("steer"))]
#[case::responses_instructions(
    r#"{"instructions":"steer","input":[{"role":"system","content":"other"}]}"#,
    Some("steer")
)]
#[case::system_message(
    r#"{"messages":[{"role":"user","content":"hi"},{"role":"system","content":"rules"}]}"#,
    Some("rules")
)]
#[case::developer_message(r#"{"messages":[{"role":"developer","content":[{"type":"text","text":"a"},{"type":"text","text":"b"}]}]}"#, Some("a\nb"))]
#[case::responses_input_message(
    r#"{"input":[{"role":"system","content":"rules"}]}"#,
    Some("rules")
)]
#[case::non_text_instructions_skipped(
    r#"{"instructions":3,"messages":[{"role":"system","content":"rules"}]}"#,
    Some("rules")
)]
#[case::system_message_without_text(r#"{"messages":[{"role":"system"}]}"#, Some(""))]
#[case::no_preamble(r#"{"messages":[{"role":"user","content":"hi"}]}"#, None)]
#[case::empty(r"{}", None)]
fn finds_preamble(#[case] source: &str, #[case] expected: Option<&str>) {
    let found = system_text(&object(source)).unwrap();
    assert_eq!(found.as_deref(), expected);
}
