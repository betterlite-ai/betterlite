#![cfg(test)]

use rstest::rstest;

use super::first_message_has_content;

#[rstest]
#[case::text(r#"{"messages":[{"role":"user","content":"hi"}]}"#, true)]
#[case::explicit_null(r#"{"messages":[{"role":"user","content":null}]}"#, true)]
#[case::parts(
    r#"{"messages":[{"role":"user","content":[{"type":"text","text":"hi"}]}]}"#,
    true
)]
#[case::absent(
    r#"{"messages":[{"role":"assistant","tool_calls":[{"id":"c","type":"function","function":{"name":"f","arguments":"{}"}}]}]}"#,
    false
)]
#[case::only_a_later_message_has_it(
    r#"{"messages":[{"role":"assistant"},{"role":"user","content":"hi"}]}"#,
    false
)]
#[case::only_a_later_message_lacks_it(
    r#"{"messages":[{"role":"user","content":"hi"},{"role":"assistant"}]}"#,
    true
)]
#[case::empty_list(r#"{"messages":[]}"#, false)]
#[case::no_messages_key(r#"{"model":"g"}"#, false)]
#[case::message_is_not_an_object(r#"{"messages":["hi"]}"#, false)]
#[case::not_json("{", false)]
fn only_a_content_key_on_the_first_message_counts(#[case] body: &str, #[case] expected: bool) {
    assert_eq!(first_message_has_content(body.as_bytes()), expected);
}
