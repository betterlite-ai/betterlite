use rstest::rstest;

use crate::arguments::json::Json;
use crate::cache_placement::insert::mark_message;
use crate::cache_placement::tests::support::{json, wire};
use crate::message_content::Message;

const CONTROL: &str = r#"{"type": "ephemeral"}"#;

fn marked(wire_text: &str, control_text: &str, openai_dialect: bool) -> Json {
    let message: Message = serde_json::from_str(wire_text).unwrap();
    wire(&mark_message(&message, &json(control_text), openai_dialect))
}

#[rstest]
#[case::user(
    r#"{"role": "user", "content": "hi"}"#,
    r#"{"role": "user", "content": "hi", "cache_control": {"type": "ephemeral"}}"#
)]
#[case::assistant(
    r#"{"role": "assistant", "content": "hi"}"#,
    r#"{"role": "assistant", "content": "hi", "cache_control": {"type": "ephemeral"}}"#
)]
fn anthropic_marks_string_content_at_the_message_top_level(
    #[case] input: &str,
    #[case] expected: &str,
) {
    assert_eq!(marked(input, CONTROL, false), json(expected));
}

#[test]
fn anthropic_marks_the_last_content_block() {
    let input = r#"{"role": "user", "content": [{"type": "text", "text": "one"}, {"type": "text", "text": "two"}]}"#;
    let expected = json(
        r#"{"role": "user", "content": [{"type": "text", "text": "one"}, {"type": "text", "text": "two", "cache_control": {"type": "ephemeral"}}]}"#,
    );
    assert_eq!(marked(input, CONTROL, false), expected);
}

#[test]
fn anthropic_marks_the_last_block_whatever_its_type() {
    let input = r#"{"role": "user", "content": [{"type": "text", "text": "one"}, {"type": "video", "source": "x"}]}"#;
    let expected = json(
        r#"{"role": "user", "content": [{"type": "text", "text": "one"}, {"type": "video", "source": "x", "cache_control": {"type": "ephemeral"}}]}"#,
    );
    assert_eq!(marked(input, CONTROL, false), expected);
}

#[rstest]
#[case::empty_blocks(r#"{"role": "user", "content": []}"#)]
#[case::no_content(r#"{"role": "user", "name": "x"}"#)]
fn anthropic_leaves_other_shapes_alone(#[case] input: &str) {
    let expected = json(input);
    assert_eq!(marked(input, CONTROL, false), expected);
}

#[rstest]
#[case::assistant(r#"{"role": "assistant", "content": "hi"}"#)]
#[case::assistant_blocks(r#"{"role": "assistant", "content": [{"type": "text", "text": "hi"}]}"#)]
#[case::empty_blocks(r#"{"role": "user", "content": []}"#)]
#[case::no_content(r#"{"role": "user", "name": "x"}"#)]
#[case::ineligible_only(r#"{"role": "user", "content": [{"type": "video", "source": "x"}]}"#)]
fn openai_leaves_assistant_and_ineligible_messages_alone(#[case] input: &str) {
    let expected = json(input);
    assert_eq!(marked(input, CONTROL, true), expected);
}

#[test]
fn openai_turns_string_content_into_a_marked_text_block() {
    let input = r#"{"role": "user", "content": "hi"}"#;
    let expected = json(
        r#"{"role": "user", "content": [{"type": "text", "text": "hi", "prompt_cache_breakpoint": {"mode": "explicit"}}]}"#,
    );
    assert_eq!(marked(input, CONTROL, true), expected);
}

#[rstest]
#[case::text("text")]
#[case::image("image")]
#[case::image_url("image_url")]
#[case::file("file")]
#[case::input_audio("input_audio")]
#[case::input_text("input_text")]
#[case::input_image("input_image")]
#[case::input_file("input_file")]
fn openai_accepts_every_declared_block_type(#[case] kind: &str) {
    let input = format!(r#"{{"role": "user", "content": [{{"type": "{kind}"}}]}}"#);
    let expected = json(&format!(
        r#"{{"role": "user", "content": [{{"type": "{kind}", "prompt_cache_breakpoint": {{"mode": "explicit"}}}}]}}"#
    ));
    assert_eq!(marked(&input, CONTROL, true), expected);
}

#[rstest]
#[case::video("video")]
#[case::pdf("pdf")]
#[case::text_document("text_document")]
fn openai_rejects_other_block_types(#[case] kind: &str) {
    let input = format!(r#"{{"role": "user", "content": [{{"type": "{kind}"}}]}}"#);
    let expected = json(&input);
    assert_eq!(marked(&input, CONTROL, true), expected);
}

#[test]
fn openai_marks_the_last_eligible_block_not_the_last_block() {
    let input = r#"{"role": "user", "content": [{"type": "text", "text": "one"}, {"type": "video", "source": "x"}]}"#;
    let expected = json(
        r#"{"role": "user", "content": [{"type": "text", "text": "one", "prompt_cache_breakpoint": {"mode": "explicit"}}, {"type": "video", "source": "x"}]}"#,
    );
    assert_eq!(marked(input, CONTROL, true), expected);
}

#[test]
fn a_mark_keeps_the_rest_of_the_message() {
    let input = r#"{"role": "user", "content": [{"type": "text", "text": "one"}], "name": "alice", "tool_call_id": "call_1"}"#;
    let expected = json(
        r#"{"role": "user", "content": [{"type": "text", "text": "one", "cache_control": {"type": "ephemeral"}}], "name": "alice", "tool_call_id": "call_1"}"#,
    );
    assert_eq!(marked(input, CONTROL, false), expected);
}

#[test]
fn the_point_control_reaches_the_message() {
    let input = r#"{"role": "user", "content": "hi"}"#;
    let expected = json(
        r#"{"role": "user", "content": "hi", "cache_control": {"type": "ephemeral", "ttl": "1h"}}"#,
    );
    assert_eq!(
        marked(input, r#"{"type": "ephemeral", "ttl": "1h"}"#, false),
        expected
    );
}
