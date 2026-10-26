#![cfg(test)]

use super::*;
use serde_json::json;

#[test]
fn counts_characters_for_budget() {
    let short = json!({"content": "\u{754c}\n".repeat(50)});
    assert_eq!(truncate_message(&short, 0), short);
    let lines = (0_usize..20_usize)
        .map(|index| format!("{index:02}{}", "\u{754c}".repeat(17)))
        .collect::<Vec<_>>();
    let message = json!({"content": lines.join("\n")});
    let expected = format!(
        "{}\n...[truncated for context window]...\n{}",
        lines.iter().take(4).cloned().collect::<Vec<_>>().join("\n"),
        lines
            .iter()
            .skip(18)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
    assert_eq!(truncate_message(&message, 40), json!({"content": expected}));
}

#[test]
fn keeps_line_based_head_and_tail() {
    let content = (0_usize..20_usize).map(|index| format!("line {index:02} 123456789"));
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
