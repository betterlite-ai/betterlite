#![cfg(test)]

mod guarded_cases;
mod orphan_policy_cases;
mod orphan_text_cases;
mod support;
mod system_cache_cases;
mod system_placement_cases;
mod system_role_cases;

use rstest::rstest;

use crate::converse_messages::{ConverseMessagePolicy, transform_messages};

use self::support::{
    CLAUDE_3_5, MODEL_COST, blocks, expected, messages, params, render, render_params,
};

#[rstest]
#[case::consecutive_guarded_user_text(guarded_cases::consecutive_guarded_user_text)]
#[case::orphan_tool_with_modify_params(orphan_policy_cases::orphan_tool_with_modify_params)]
#[case::orphan_tool_error(orphan_policy_cases::orphan_tool_error)]
#[case::system_around_tool_result(system_placement_cases::system_around_tool_result)]
#[case::system_role_as_user(system_role_cases::system_role_as_user)]
#[case::system_text_cache_control(system_cache_cases::system_text_cache_control)]
#[case::orphan_neutralization_default(orphan_policy_cases::orphan_neutralization_default)]
#[case::orphan_text_projection(orphan_text_cases::orphan_text_projection)]
fn converse_messages_cases(#[case] case: fn()) {
    case();
}

const HISTORY: &str = r#"[
    {"role": "system", "content": "rules"},
    {"role": "assistant", "content": "calling", "tool_calls": [{"id": "call_1", "type": "function", "function": {"name": "lookup", "arguments": "{}"}}]},
    {"role": "tool", "tool_call_id": "call_1", "content": "result"},
    {"role": "user", "content": "thanks"}
]"#;

const GUARDED_PIPELINE: &str = r#"[
    {"role": "assistant", "content": "calling\n[tool call call_1: lookup({})]"},
    {"role": "user", "content": [{"type": "guarded_text", "text": "[tool result for call_1: result]"}]},
    {"role": "user", "content": [{"type": "guarded_text", "text": "thanks"}]}
]"#;

const UNCHANGED_PIPELINE: &str = r#"[
    {"role": "assistant", "content": "calling", "tool_calls": [{"id": "call_1", "type": "function", "function": {"name": "lookup", "arguments": "{}"}}]},
    {"role": "tool", "tool_call_id": "call_1", "content": "result"},
    {"role": "user", "content": "thanks"}
]"#;

const DUMMY_TOOLS: &str = r#"{"tools": [{"type": "function", "function": {"name": "dummy_tool", "description": "This is a dummy tool call", "parameters": {"type": "object", "properties": {}}}}]}"#;

#[test]
fn the_pipeline_hoists_the_system_prefix_and_wraps_guarded_results() {
    let mut optional = params(r#"{"guardrailConfig": null}"#);
    let model_cost = params(MODEL_COST);
    let policy = ConverseMessagePolicy {
        model: Some(CLAUDE_3_5),
        model_cost: &model_cost,
        neutralize: true,
        modify_params: false,
    };
    let transformed = transform_messages(&messages(HISTORY), &mut optional, &policy)
        .expect("the messages handle");
    assert_eq!(
        blocks(&transformed.system),
        expected(r#"[{"text": "rules"}]"#)
    );
    assert_eq!(render(&transformed.messages), expected(GUARDED_PIPELINE));
    assert!(optional.contains_key("guardrailConfig"));
}

#[test]
fn the_pipeline_adds_dummy_tools_when_neutralization_is_off() {
    let mut optional = params("{}");
    let model_cost = params(MODEL_COST);
    let policy = ConverseMessagePolicy {
        model: Some(CLAUDE_3_5),
        model_cost: &model_cost,
        neutralize: false,
        modify_params: true,
    };
    let transformed = transform_messages(&messages(HISTORY), &mut optional, &policy)
        .expect("the messages handle");
    assert_eq!(render(&transformed.messages), expected(UNCHANGED_PIPELINE));
    assert_eq!(render_params(&optional), expected(DUMMY_TOOLS));
}
