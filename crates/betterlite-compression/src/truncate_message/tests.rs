#![cfg(test)]

use super::*;
use serde_json::json;

#[test]
fn keeps_line_based_head_and_tail() {
    let content = (0..20).map(|index| format!("line {index:02} 123456789"));
    let result = truncate_message(
        &json!({"role":"user","content":content.collect::<Vec<_>>().join("\n")}),
        40,
    );
    let text = result["content"].as_str().unwrap_or_default();
    assert!(text.contains("line 00"));
    assert!(text.contains("line 19"));
    assert!(text.contains("line 17"));
    assert!(!text.contains("line 16"));
    assert!(text.contains("truncated"));
}
