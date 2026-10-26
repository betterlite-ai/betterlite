#![cfg(test)]

use super::*;
use serde_json::json;

#[test]
fn stub_preserves_role_and_line_count() {
    let result = stub_message(&json!({"role":"user","content":"one\ntwo"}), "main.rs");
    assert_eq!(result["role"], "user");
    assert!(
        result["content"]
            .as_str()
            .is_some_and(|text| text.contains("2 lines"))
    );
}

#[test]
fn counts_the_line_after_a_trailing_newline() {
    let stub = stub_message(&json!({"role":"user","content":"one\n"}), "note");
    assert!(
        stub["content"]
            .as_str()
            .unwrap_or_default()
            .contains("2 lines")
    );
}
