#![cfg(test)]

use betterlite_models::decisions::Json;
use rstest::rstest;

use super::{ChatRole, content_text};

#[rstest]
#[case::human("human", Some(ChatRole::User))]
#[case::user("user", Some(ChatRole::User))]
#[case::ai("ai", Some(ChatRole::Assistant))]
#[case::assistant("assistant", Some(ChatRole::Assistant))]
#[case::system("system", Some(ChatRole::System))]
#[case::tool("tool", Some(ChatRole::Tool))]
#[case::unknown("bot", None)]
#[case::empty("", None)]
fn maps_openai_roles_and_langchain_types(#[case] name: &str, #[case] expected: Option<ChatRole>) {
    assert_eq!(ChatRole::from_name(name), expected);
}

#[rstest]
#[case::null("null", "")]
#[case::text(r#""s""#, "s")]
#[case::int("1", "1")]
#[case::float("1.5", "1.5")]
#[case::bool("true", "true")]
#[case::object("{\"b\": 1, \"a\": \"\u{e9}\"}", concat!(r#"{"b": 1, "a": ""#, "\\", r#"u00e9"}"#))]
#[case::empty_blocks("[]", "")]
#[case::text_blocks(
    r#"[{"type": "text", "text": "a"}, {"type": "thinking"}, {"text": "b", "extra": 1}]"#,
    "a\n\nb"
)]
#[case::text_block_without_text(
    r#"[{"type": "text", "text": null}]"#,
    r#"[{"type": "text", "text": null}]"#
)]
#[case::reasoning_without_text(r#"[{"type": "reasoning", "text": null}]"#, "")]
#[case::not_a_block(r#"["x"]"#, r#"["x"]"#)]
#[case::numeric_type(r#"[{"type": 1, "text": "a"}]"#, r#"[{"type": 1, "text": "a"}]"#)]
#[case::numeric_text(r#"[{"type": "text", "text": 1}]"#, r#"[{"type": "text", "text": 1}]"#)]
#[case::null_type(r#"[{"type": null, "text": "a"}]"#, r#"[{"type": null, "text": "a"}]"#)]
fn content_text_keeps_text_blocks_and_dumps_the_rest(
    #[case] content: &str,
    #[case] expected: &str,
) {
    let parsed: Json = serde_json::from_str(content).unwrap();
    assert_eq!(content_text(&parsed), expected);
}
