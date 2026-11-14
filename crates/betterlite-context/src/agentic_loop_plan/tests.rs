#![cfg(test)]
#![cfg_attr(
    test,
    expect(
        clippy::disallowed_methods,
        clippy::disallowed_types,
        reason = "the plan fixtures state the JSON LiteLLM's pydantic models dump, field for field"
    )
)]

use rstest::rstest;
use serde::Serialize;
use serde_json::json;
use serde_json::value::to_raw_value;

use super::{AgenticLoopPlan, AgenticLoopRequestPatch, FollowupMessage, JsonMap, get_text};

#[derive(Serialize)]
struct ToolMessage {
    role: &'static str,
    content: &'static str,
}

fn map(value: serde_json::Value) -> JsonMap {
    serde_json::from_value(value).unwrap()
}

#[test]
fn default_plan_runs_nothing() {
    let plan = AgenticLoopPlan::default();
    assert!(!plan.run_agentic_loop);
    assert!(plan.request_patch.is_none());
    assert!(plan.response_override.is_none());
    assert!(!plan.terminate);
    assert!(plan.stop_reason.is_none());
    assert!(plan.metadata.is_empty());
}

#[test]
fn default_patch_is_empty() {
    let patch = AgenticLoopRequestPatch::default();
    assert!(patch.model.is_none());
    assert!(patch.messages.is_none());
    assert!(patch.tools.is_none());
    assert!(patch.max_tokens.is_none());
    assert!(patch.optional_params.is_empty());
    assert!(patch.kwargs.is_empty());
}

#[test]
fn plan_serializes_litellm_field_names() {
    let value = serde_json::to_value(AgenticLoopPlan::default()).unwrap();
    assert_eq!(
        value,
        json!({
            "run_agentic_loop": false,
            "request_patch": null,
            "response_override": null,
            "terminate": false,
            "stop_reason": null,
            "metadata": {},
        })
    );
}

#[test]
fn patch_serializes_litellm_field_names() {
    let value = serde_json::to_value(AgenticLoopRequestPatch::default()).unwrap();
    assert_eq!(
        value,
        json!({
            "model": null,
            "messages": null,
            "tools": null,
            "max_tokens": null,
            "optional_params": {},
            "kwargs": {},
        })
    );
}

#[test]
fn plan_deserializes_absent_fields_as_defaults() {
    let plan: AgenticLoopPlan = serde_json::from_value(json!({})).unwrap();
    assert!(!plan.run_agentic_loop);
    assert!(plan.request_patch.is_none());
    assert!(!plan.terminate);
    assert!(plan.metadata.is_empty());
}

#[test]
fn patch_deserializes_absent_fields_as_defaults() {
    let patch: AgenticLoopRequestPatch = serde_json::from_value(json!({})).unwrap();
    assert!(patch.model.is_none());
    assert!(patch.optional_params.is_empty());
    assert!(patch.kwargs.is_empty());
}

#[test]
fn plan_round_trips_a_full_value() {
    let value = json!({
        "run_agentic_loop": true,
        "request_patch": {
            "model": "azure/gpt-4o",
            "messages": [{"role": "user", "content": "hi"}],
            "tools": null,
            "max_tokens": 512_i64,
            "optional_params": {"temperature": 0.2_f64},
            "kwargs": {"litellm_call_id": "call-1"},
        },
        "response_override": {"id": "resp-1"},
        "terminate": true,
        "stop_reason": "web_search_failed",
        "metadata": {"tool_type": "websearch"},
    });
    let plan: AgenticLoopPlan = serde_json::from_value(value.clone()).unwrap();
    assert_eq!(serde_json::to_value(plan).unwrap(), value);
}

#[test]
fn followup_message_keeps_the_json_it_wraps() {
    let raw = to_raw_value(&ToolMessage {
        role: "tool",
        content: "results",
    })
    .unwrap();
    assert_eq!(
        serde_json::to_value(FollowupMessage(raw)).unwrap(),
        json!({"role": "tool", "content": "results"})
    );
}

#[test]
fn followup_message_is_transparent() {
    let raw = to_raw_value(&json!({"role": "assistant"})).unwrap();
    let message = FollowupMessage(raw);
    assert_eq!(
        serde_json::to_value(message).unwrap(),
        json!({"role": "assistant"})
    );
    let decoded: FollowupMessage = serde_json::from_value(json!({"role": "user"})).unwrap();
    assert_eq!(decoded.0.get(), "{\"role\":\"user\"}");
}

#[rstest]
#[case::string(json!({"query": "weather"}), Some("weather".to_owned()))]
#[case::empty_string(json!({"query": ""}), Some(String::new()))]
#[case::number(json!({"query": 3_i32}), None)]
#[case::null(json!({"query": null}), None)]
#[case::object(json!({"query": {"text": "weather"}}), None)]
#[case::missing(json!({"other": "weather"}), None)]
fn get_text_reads_json_strings_only(
    #[case] value: serde_json::Value,
    #[case] expected: Option<String>,
) {
    assert_eq!(get_text(&map(value), "query"), expected);
}
