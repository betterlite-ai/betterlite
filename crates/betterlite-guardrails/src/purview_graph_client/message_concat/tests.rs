#![cfg(test)]

use betterlite_prompts::message_content::Message;
use rstest::rstest;

use super::prompt_text_for_dlp;

fn messages(raw: &str) -> Vec<Message> {
    serde_json::from_str(raw).unwrap()
}

#[rstest]
#[case::no_messages("[]", None)]
#[case::one_message(r#"[{"role": "user", "content": "  hello  "}]"#, Some("hello"))]
#[case::every_role_joined_by_a_blank_line(
    r#"[{"role": "system", "content": "be nice"}, {"role": "user", "content": "my card"}, {"role": "assistant", "content": "ok"}]"#,
    Some("be nice\n\nmy card\n\nok")
)]
#[case::text_parts_are_read(
    r#"[{"role": "user", "content": [{"type": "text", "text": "part one "}, {"type": "text", "text": "part two"}]}]"#,
    Some("part one part two")
)]
#[case::tool_call_arguments_follow_the_content(
    r#"[{"role": "assistant", "content": "calling", "tool_calls": [{"function": {"arguments": "{\"ssn\": \"1\"}"}}]}]"#,
    Some("calling\n{\"ssn\": \"1\"}")
)]
#[case::tool_call_without_content(
    r#"[{"role": "assistant", "content": null, "tool_calls": [{"function": {"arguments": "args"}}]}]"#,
    Some("args")
)]
#[case::legacy_function_call(
    r#"[{"role": "assistant", "content": "x", "function_call": {"name": "f", "arguments": "legacy"}}]"#,
    Some("x\nlegacy")
)]
#[case::empty_messages_are_skipped(
    r#"[{"role": "user", "content": "first"}, {"role": "assistant", "content": "  "}, {"role": "user", "content": "last"}]"#,
    Some("first\n\nlast")
)]
#[case::only_empty_messages(r#"[{"role": "user", "content": ""}, {"role": "tool"}]"#, None)]
fn message_concat_joins_every_message_for_dlp(#[case] raw: &str, #[case] expected: Option<&str>) {
    assert_eq!(prompt_text_for_dlp(&messages(raw)).as_deref(), expected);
}
