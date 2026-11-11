#![cfg(test)]

use serde_json::{Value, json};

use crate::helpers::Helper as _;

use super::Op;

fn run(input: &Value) -> Value {
    Op::decode_and_run(input).expect("helper")
}

const CONVERSATION: &str = r#"[
    {"role":"assistant","content":[
        {"type":"tool_use","id":"a"},
        {"type":"tool_use","id":"b"},
        {"type":"tool_use","id":"c"},
        {"type":"tool_use","id":"d"}
    ]},
    {"role":"user","content":[
        {"type":"tool_result","tool_use_id":"a","content":"one"},
        {"type":"tool_result","tool_use_id":"b","content":"two"},
        {"type":"tool_result","tool_use_id":"c","content":"three"},
        {"type":"tool_result","tool_use_id":"d","content":"four"}
    ]}
]"#;

#[test]
fn scripted_editor_clears_and_records_calls() {
    let messages: Value = serde_json::from_str(CONVERSATION).expect("messages");
    let value = run(&json!({
        "action": "editor",
        "model": "gpt-4o-mini",
        "messages": messages,
        "tools": null,
        "system": null,
        "edit_spec": {},
        "counter": {"mode": "scripted", "values": [100_001_u64, 80_u64]},
    }));
    assert_eq!(value["applied"]["cleared_tool_uses"], json!(1_u64));
    assert_eq!(value["applied"]["cleared_input_tokens"], json!(99_921_u64));
    assert_eq!(value["calls"].as_array().map(Vec::len), Some(2));
    assert_eq!(value["calls"][0]["count"], json!(100_001_u64));
    assert_eq!(value["calls"][1]["count"], json!(80_u64));
}

#[test]
fn ignored_knobs_are_reported_in_order() {
    let messages: Value = serde_json::from_str(CONVERSATION).expect("messages");
    let value = run(&json!({
        "action": "editor",
        "model": "gpt-4o-mini",
        "messages": messages,
        "edit_spec": {
            "clear_at_least": null,
            "exclude_tools": ["lookup"],
            "clear_tool_inputs": false,
            "trigger": {"value": 999_999_u64}
        },
        "counter": {"mode": "scripted", "values": [10_u64]},
    }));
    assert_eq!(value["applied"], Value::Null);
    assert_eq!(
        value["warnings"],
        json!([
            "context_management polyfill: ignoring 'clear_at_least' on clear_tool_uses_20250919 (supported only on Anthropic-family forwarding path in v0)",
            "context_management polyfill: ignoring 'exclude_tools' on clear_tool_uses_20250919 (supported only on Anthropic-family forwarding path in v0)",
            "context_management polyfill: ignoring 'clear_tool_inputs' on clear_tool_uses_20250919 (supported only on Anthropic-family forwarding path in v0)"
        ])
    );
}

#[test]
fn visible_edits_hide_a_bare_compaction_edit() {
    let value = run(&json!({
        "action": "visible_edits",
        "applied_edits": [
            {"type": "compact_20260112"},
            {"type": "clear_tool_uses_20250919"},
            {}
        ],
        "compaction_block": null,
    }));
    assert_eq!(
        value["visible"],
        json!([{"type": "clear_tool_uses_20250919"}, {}])
    );
}

#[test]
fn context_management_error_carries_its_status() {
    let failure = Op::decode_and_run(&json!({
        "action": "visible_edits",
        "error": {"status_code": 403_u16, "message": "denied"},
    }))
    .expect_err("error");
    assert_eq!(failure.status, Some(403));
    assert_eq!(failure.message, "denied");
    assert!(failure.error_type.is_none());
    assert!(failure.param.is_none());
    assert!(failure.code.is_none());
}

#[test]
fn constants_carry_the_pinned_values() {
    let value = run(&json!({"action": "constants"}));
    assert_eq!(
        value["clear_tool_uses_edit_type"],
        json!("clear_tool_uses_20250919")
    );
    assert_eq!(value["default_input_tokens_trigger"], json!(100_000_u64));
    assert_eq!(
        value["cleared_tool_result_content"],
        json!("[Cleared by context management]")
    );
    assert_eq!(
        value["compact_no_tool_calls_suffix"],
        json!(" Do not call any tools while writing this summary; respond with text only.")
    );
}
