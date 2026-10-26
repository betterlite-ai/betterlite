#![cfg(test)]

use super::*;
use serde_json::json;

#[test]
fn keeps_line_based_head_and_tail() {
    let content = (0..100).map(|index| format!("line {index}"));
    let result = truncate_message(
        &json!({"role":"user","content":content.collect::<Vec<_>>().join("\n")}),
        10,
    );
    let text = result["content"].as_str().unwrap_or_default();
    assert!(text.contains("line 0"));
    assert!(text.contains("line 99"));
    assert!(text.contains("truncated"));
}
