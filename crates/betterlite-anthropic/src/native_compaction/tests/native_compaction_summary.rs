#![cfg(test)]

use rstest::rstest;

use crate::native_compaction::anthropic::AnthropicCompaction;
use crate::native_compaction::error::NativeCompactionError;
use crate::native_compaction::protocol::{CompactionProtocol, NativeCompactionProvider as _};

use super::fixtures::{FakeLookup, mapping};

fn messages_reply(blocks: &str) -> String {
    format!(r#"{{"stop_reason":"compaction","content":{blocks}}}"#)
}

fn chat_reply(blocks: &str) -> String {
    format!(
        r#"{{"choices":[{{"message":{{"provider_specific_fields":{{"compaction_blocks":{blocks}}}}}}}]}}"#
    )
}

fn summary(
    protocol: CompactionProtocol,
    reply: &str,
) -> Result<Option<String>, NativeCompactionError> {
    let lookup = FakeLookup::answering(Ok(Some(true)));
    let provider = AnthropicCompaction { lookup: &lookup };
    provider.extract_summary(protocol, &mapping(reply))
}

#[rstest]
#[case::plain(
    r#"{"type":"compaction","signature":"sig","content":"the summary"}"#,
    Some("the summary")
)]
#[case::content_is_not_stripped(
    r#"{"type":"compaction","signature":"sig","content":"  the summary\n"}"#,
    Some("  the summary\n")
)]
#[case::whitespace_signature(
    r#"{"type":"compaction","signature":" ","content":"the summary"}"#,
    Some("the summary")
)]
#[case::extra_keys_ignored(
    r#"{"type":"compaction","signature":"sig","content":"c","other":1}"#,
    Some("c")
)]
#[case::empty_signature(r#"{"type":"compaction","signature":"","content":"c"}"#, None)]
#[case::null_signature(r#"{"type":"compaction","signature":null,"content":"c"}"#, None)]
#[case::numeric_signature(r#"{"type":"compaction","signature":1,"content":"c"}"#, None)]
#[case::missing_signature(r#"{"type":"compaction","content":"c"}"#, None)]
#[case::other_type(r#"{"type":"text","signature":"sig","content":"c"}"#, None)]
#[case::missing_type(r#"{"signature":"sig","content":"c"}"#, None)]
#[case::numeric_type(r#"{"type":1,"signature":"sig","content":"c"}"#, None)]
#[case::empty_content(r#"{"type":"compaction","signature":"sig","content":""}"#, None)]
#[case::spaces_content(
    r#"{"type":"compaction","signature":"sig","content":"  \t\r\n"}"#,
    None
)]
#[case::no_break_space_content(
    r#"{"type":"compaction","signature":"sig","content":"\u00a0"}"#,
    None
)]
#[case::separator_content(
    r#"{"type":"compaction","signature":"sig","content":"\u001c\u001d\u001e\u001f"}"#,
    None
)]
#[case::separator_then_text(
    r#"{"type":"compaction","signature":"sig","content":"\u001fx"}"#,
    Some("\u{1f}x")
)]
#[case::next_line_content(
    r#"{"type":"compaction","signature":"sig","content":"\u0085 \u2003"}"#,
    None
)]
#[case::null_content(r#"{"type":"compaction","signature":"sig","content":null}"#, None)]
#[case::list_content(r#"{"type":"compaction","signature":"sig","content":["c"]}"#, None)]
#[case::missing_content(r#"{"type":"compaction","signature":"sig"}"#, None)]
fn native_compaction_summary_checks_the_single_block(
    #[case] block: &str,
    #[case] expected: Option<&str>,
) {
    let blocks = format!("[{block}]");

    let from_messages = summary(CompactionProtocol::Messages, &messages_reply(&blocks)).unwrap();
    let from_chat = summary(CompactionProtocol::Chat, &chat_reply(&blocks)).unwrap();

    assert_eq!(from_messages.as_deref(), expected);
    assert_eq!(from_chat.as_deref(), expected);
}

#[rstest]
#[case::no_blocks("[]")]
#[case::two_blocks(
    r#"[{"type":"compaction","signature":"s","content":"a"},{"type":"compaction","signature":"s","content":"b"}]"#
)]
fn native_compaction_summary_needs_exactly_one_block(#[case] blocks: &str) {
    let from_messages = summary(CompactionProtocol::Messages, &messages_reply(blocks)).unwrap();
    let from_chat = summary(CompactionProtocol::Chat, &chat_reply(blocks)).unwrap();

    assert_eq!(from_messages, None);
    assert_eq!(from_chat, None);
}

#[test]
fn native_compaction_summary_passes_block_type_errors_up() {
    let error = summary(CompactionProtocol::Messages, &messages_reply(r#"["x"]"#)).unwrap_err();

    assert!(matches!(
        error,
        NativeCompactionError::NotAnObjectList { field: "content" }
    ));
}
