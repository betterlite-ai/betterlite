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
fn unknown_role_uses_tool_call_default() {
    assert_eq!(role_for("other", true), ChatRole::Assistant);
}
