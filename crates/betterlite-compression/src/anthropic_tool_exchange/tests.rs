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

#[test]
fn validates_multiple_pairs_and_all_tool_ids() {
    let messages = vec![
        json!({"role":"user","content":"before"}),
        json!({"role":"assistant","content":[{"type":"tool_use","id":"a"}]}),
        json!({"role":"user","content":[{"type":"tool_result","tool_use_id":"a"}]}),
        json!({"role":"assistant","content":[{"type":"tool_use","id":"b"}]}),
        json!({"role":"user","content":[{"type":"tool_result","tool_use_id":"b"}]}),
    ];
    assert_eq!(
        extract_exchange_spans(&messages).0,
        vec![vec![1, 2], vec![3, 4]]
    );
    let incomplete = vec![
        json!({"role":"assistant","content":[{"type":"tool_use","id":"a"},{"type":"tool_use","id":"b"}]}),
        json!({"role":"user","content":[{"type":"tool_result","tool_use_id":"a"}]}),
    ];
    assert_eq!(
        extract_exchange_spans(&incomplete).1,
        Some("invalid_anthropic_tool_sequence")
    );
}

#[test]
fn ignores_empty_ids_and_rejects_non_user_follow_up() {
    let empty_use = vec![
        json!({"role":"assistant","content":[{"type":"tool_use","id":""},{"type":"text","id":"t"}]}),
        json!({"role":"assistant","content":"next"}),
    ];
    assert_eq!(extract_exchange_spans(&empty_use), (Vec::new(), None));
    let empty_result = vec![
        json!({"role":"assistant","content":[{"type":"tool_use","id":"a"}]}),
        json!({"role":"user","content":[{"type":"tool_result","tool_use_id":""},{"type":"text","tool_use_id":"a"}]}),
    ];
    assert_eq!(
        extract_exchange_spans(&empty_result),
        (Vec::new(), Some("invalid_anthropic_tool_sequence"))
    );
    let wrong_role = vec![
        json!({"role":"assistant","content":[{"type":"tool_use","id":"a"}]}),
        json!({"role":"assistant","content":[{"type":"tool_result","tool_use_id":"a"}]}),
    ];
    assert_eq!(
        extract_exchange_spans(&wrong_role),
        (Vec::new(), Some("invalid_anthropic_tool_sequence"))
    );
}

#[test]
fn skips_assistant_messages_without_tool_use() {
    let messages = vec![
        json!({"role":"assistant","content":[{"type":"text","text":"plain"}]}),
        json!({"role":"user","content":"next"}),
        json!({"role":"assistant","content":[{"type":"tool_use","id":"a"}]}),
        json!({"role":"user","content":[{"type":"tool_result","tool_use_id":"a"}]}),
    ];
    assert_eq!(extract_exchange_spans(&messages).0, vec![vec![2, 3]]);
}
