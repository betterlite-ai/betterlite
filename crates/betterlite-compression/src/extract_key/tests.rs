#![cfg(test)]

use super::*;
use serde_json::json;

#[test]
fn extracts_filename_and_suffixes_duplicates() {
    let message = json!({"content":"# src/main.rs\ncode"});
    let mut used = Vec::new();
    assert_eq!(extract_key(&message, 4, &mut used), "main.rs");
    assert_eq!(extract_key(&message, 4, &mut used), "main.rs_2");
}

#[test]
fn falls_back_to_message_index() {
    let mut used = Vec::new();
    assert_eq!(
        extract_key(&json!({"content":"hello"}), 7, &mut used),
        "message_7"
    );
}
