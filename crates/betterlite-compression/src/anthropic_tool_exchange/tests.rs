#![cfg(test)]

use super::*;
use serde_json::json;

#[test]
fn validates_immediate_complete_pair() {
    let messages = vec![
        json!({"role":"assistant","content":[{"type":"tool_use","id":"a"}]}),
        json!({"role":"user","content":[{"type":"tool_result","tool_use_id":"a"}]}),
    ];
    assert_eq!(extract_exchange_spans(&messages), (vec![vec![0, 1]], None));
}

#[test]
fn rejects_missing_result_without_panicking() {
    let messages = vec![
        json!({"role":"assistant","content":[{"type":"tool_use","id":"a"}]}),
        json!({"role":"user","content":null}),
    ];
    assert_eq!(
        extract_exchange_spans(&messages).1,
        Some("invalid_anthropic_tool_sequence")
    );
}
