#![cfg(test)]

use super::*;
use serde_json::json;

#[test]
fn extracts_filename_and_suffixes_duplicates() {
    let message = json!({"content":"# src/main.rs\ncode"});
    let mut used = Vec::new();
    assert_eq!(extract_key(&message, 4, &mut used), "main.rs");
    assert_eq!(extract_key(&message, 4, &mut used), "main.rs_2");
    assert_eq!(extract_key(&message, 4, &mut used), "main.rs_3");
}

#[test]
fn content_blocks_contribute_text_to_key_search() {
    let message = json!({"content":[{"type":"text","text":"File: src/config.yaml"}]});
    let mut used = Vec::new();
    assert_eq!(extract_key(&message, 0, &mut used), "config.yaml");
}

#[test]
fn falls_back_to_message_index() {
    let mut used = Vec::new();
    assert_eq!(
        extract_key(&json!({"content":"hello"}), 7, &mut used),
        "message_7"
    );
}
