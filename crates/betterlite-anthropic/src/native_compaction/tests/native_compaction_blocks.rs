#![cfg(test)]

use rstest::rstest;

use crate::native_compaction::blocks::native_blocks;
use crate::native_compaction::error::NativeCompactionError;
use crate::native_compaction::protocol::CompactionProtocol;

use super::fixtures::mapping;

const BLOCK: &str = r#"{"type":"compaction","signature":"sig","content":"text"}"#;

fn messages_reply(stop_reason: &str, content: &str) -> String {
    format!(r#"{{"stop_reason":{stop_reason},"content":{content}}}"#)
}

fn chat_reply(choices: &str) -> String {
    format!(r#"{{"choices":{choices}}}"#)
}

fn chat_choice(blocks: &str) -> String {
    format!(r#"{{"message":{{"provider_specific_fields":{{"compaction_blocks":{blocks}}}}}}}"#)
}

#[rstest]
#[case::stopped_with_block(CompactionProtocol::Messages, messages_reply(r#""compaction""#, &format!("[{BLOCK}]")), 1)]
#[case::stopped_with_two_blocks(CompactionProtocol::Messages, messages_reply(r#""compaction""#, &format!("[{BLOCK},{BLOCK}]")), 2)]
#[case::stopped_with_no_content(CompactionProtocol::Messages, r#"{"stop_reason":"compaction"}"#.to_owned(), 0)]
#[case::stopped_with_null_content(
    CompactionProtocol::Messages,
    messages_reply(r#""compaction""#, "null"),
    0
)]
#[case::other_stop_reason(CompactionProtocol::Messages, messages_reply(r#""end_turn""#, &format!("[{BLOCK}]")), 0)]
#[case::null_stop_reason(CompactionProtocol::Messages, messages_reply("null", &format!("[{BLOCK}]")), 0)]
#[case::no_stop_reason(CompactionProtocol::Messages, format!(r#"{{"content":[{BLOCK}]}}"#), 0)]
#[case::one_choice(CompactionProtocol::Chat, chat_reply(&format!("[{}]", chat_choice(&format!("[{BLOCK}]")))), 1)]
#[case::two_blocks(CompactionProtocol::Chat, chat_reply(&format!("[{}]", chat_choice(&format!("[{BLOCK},{BLOCK}]")))), 2)]
#[case::two_choices(CompactionProtocol::Chat,
    chat_reply(&format!("[{0},{0}]", chat_choice(&format!("[{BLOCK}]")))),
    0
)]
#[case::no_choices(CompactionProtocol::Chat, chat_reply("[]"), 0)]
#[case::choices_absent(CompactionProtocol::Chat, "{}".to_owned(), 0)]
#[case::choices_null(CompactionProtocol::Chat, chat_reply("null"), 0)]
#[case::choice_without_message(CompactionProtocol::Chat, chat_reply("[{}]"), 0)]
#[case::message_null(CompactionProtocol::Chat, chat_reply(r#"[{"message":null}]"#), 0)]
#[case::message_without_fields(CompactionProtocol::Chat, chat_reply(r#"[{"message":{}}]"#), 0)]
#[case::fields_null(
    CompactionProtocol::Chat,
    chat_reply(r#"[{"message":{"provider_specific_fields":null}}]"#),
    0
)]
#[case::fields_without_blocks(
    CompactionProtocol::Chat,
    chat_reply(r#"[{"message":{"provider_specific_fields":{}}}]"#),
    0
)]
#[case::blocks_null(CompactionProtocol::Chat, chat_reply(&format!("[{}]", chat_choice("null"))), 0)]
fn native_compaction_blocks_count_what_the_protocol_reads(
    #[case] protocol: CompactionProtocol,
    #[case] reply: String,
    #[case] count: usize,
) {
    let response = mapping(&reply);

    let blocks = native_blocks(protocol, &response).unwrap();

    assert_eq!(blocks.len(), count);
}

#[rstest]
#[case::content_not_a_list(
    CompactionProtocol::Messages,
    messages_reply(r#""compaction""#, r#""text""#),
    "content"
)]
#[case::content_is_an_object(
    CompactionProtocol::Messages,
    messages_reply(r#""compaction""#, "{}"),
    "content"
)]
#[case::content_item_is_text(
    CompactionProtocol::Messages,
    messages_reply(r#""compaction""#, r#"["x"]"#),
    "content"
)]
#[case::content_item_is_a_list(
    CompactionProtocol::Messages,
    messages_reply(r#""compaction""#, "[[]]"),
    "content"
)]
#[case::content_item_is_null(
    CompactionProtocol::Messages,
    messages_reply(r#""compaction""#, "[null]"),
    "content"
)]
#[case::choices_not_a_list(CompactionProtocol::Chat, r#"{"choices":{}}"#.to_owned(), "choices")]
#[case::choice_not_an_object(CompactionProtocol::Chat, chat_reply("[1]"), "choices")]
#[case::message_not_an_object(
    CompactionProtocol::Chat,
    chat_reply(r#"[{"message":"x"}]"#),
    "message"
)]
#[case::fields_not_an_object(
    CompactionProtocol::Chat,
    chat_reply(r#"[{"message":{"provider_specific_fields":[]}}]"#),
    "provider_specific_fields"
)]
#[case::blocks_not_a_list(CompactionProtocol::Chat, chat_reply(&format!("[{}]", chat_choice("{}"))), "compaction_blocks")]
#[case::block_not_an_object(CompactionProtocol::Chat, chat_reply(&format!("[{}]", chat_choice("[true]"))), "compaction_blocks")]
fn native_compaction_blocks_wrong_types_name_the_field(
    #[case] protocol: CompactionProtocol,
    #[case] reply: String,
    #[case] field: &str,
) {
    let response = mapping(&reply);

    let error = native_blocks(protocol, &response).unwrap_err();

    assert!(matches!(
        error,
        NativeCompactionError::NotAnObject { field: named }
            | NativeCompactionError::NotAnObjectList { field: named } if named == field
    ));
}

#[test]
fn native_compaction_blocks_messages_ignore_chat_fields() {
    let response = mapping(&chat_reply(&format!(
        "[{}]",
        chat_choice(&format!("[{BLOCK}]"))
    )));

    let blocks = native_blocks(CompactionProtocol::Messages, &response).unwrap();

    assert!(blocks.is_empty());
}

#[test]
fn native_compaction_blocks_chat_ignore_messages_fields() {
    let response = mapping(&messages_reply(r#""compaction""#, &format!("[{BLOCK}]")));

    let blocks = native_blocks(CompactionProtocol::Chat, &response).unwrap();

    assert!(blocks.is_empty());
}
