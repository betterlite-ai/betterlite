#![cfg(test)]

use betterlite_prompts::arguments::json::Json;
use rstest::rstest;

use super::{conversation_messages, message_text};
use crate::straiker::v3::fixtures::{object, parsed};

#[rstest]
#[case::string_content(r#"{"content":"hi"}"#, "hi")]
#[case::text_blocks_joined_by_newline(
    r#"{"content":[{"type":"image"},{"type":"text","text":"a"},{"type":"text","text":"b"}]}"#,
    "a\nb"
)]
#[case::non_text_block_value_skipped(r#"{"content":[{"text":1},"loose",{"text":"kept"}]}"#, "kept")]
#[case::no_content(r#"{"role":"user"}"#, "")]
#[case::null_content(r#"{"content":null}"#, "")]
#[case::object_content(r#"{"content":{"text":"x"}}"#, "")]
fn reads_message_text(#[case] source: &str, #[case] expected: &str) {
    assert_eq!(message_text(&object(source)), expected);
}

#[rstest]
#[case::messages_win(
    r#"{"messages":[{"role":"user"}],"input":[{"role":"system"}]}"#,
    r#"[{"role":"user"}]"#
)]
#[case::empty_messages_fall_to_input(
    r#"{"messages":[],"input":[{"role":"system"}]}"#,
    r#"[{"role":"system"}]"#
)]
#[case::input_alone(r#"{"input":[{"role":"user"}]}"#, r#"[{"role":"user"}]"#)]
#[case::non_object_items_dropped(r#"{"messages":["x",{"role":"user"},3]}"#, r#"[{"role":"user"}]"#)]
#[case::text_messages_do_not_fall_to_input(r#"{"messages":"abc","input":[{"role":"user"}]}"#, "[]")]
#[case::text_input_has_no_messages(r#"{"input":"plain"}"#, "[]")]
#[case::neither("{}", "[]")]
fn lists_conversation_messages(#[case] source: &str, #[case] expected: &str) {
    let body = object(source);
    let found = Json::List(
        conversation_messages(&body)
            .into_iter()
            .map(|message| Json::Object(message.clone()))
            .collect(),
    );
    assert_eq!(found, parsed(expected));
}
