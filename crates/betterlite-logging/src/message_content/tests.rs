#![cfg(test)]

use super::*;

#[test]
fn plain_content_is_preserved() {
    assert_eq!(content_text(&serde_json::json!("hello")), "hello");
}

#[test]
fn response_blocks_keep_text() {
    assert_eq!(
        content_text(&serde_json::json!([{"type":"text","text":"a"},{"type":"reasoning"}])),
        "a"
    );
}

#[test]
fn invalid_and_non_text_blocks_are_json_or_empty() {
    assert_eq!(content_text(&serde_json::json!([{"type":"tool_call"}])), "");
    assert_eq!(
        content_text(&serde_json::json!([{"type":"image"}])),
        "[{\"type\":\"image\"}]"
    );
    assert_eq!(content_text(&serde_json::json!(null)), "");
}

#[test]
fn unknown_role_uses_tool_call_default() {
    assert_eq!(role_for("other", true), ChatRole::Assistant);
}
