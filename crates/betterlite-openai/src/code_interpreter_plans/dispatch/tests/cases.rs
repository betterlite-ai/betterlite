#[cfg(test)]
use std::time::Instant;

#[cfg(test)]
use serde_json::{Map, Value, json};

#[cfg(test)]
use crate::code_execution_records::SANDBOX_KEY;
#[cfg(test)]
use crate::code_interpreter_plans::gate::CHAT_COMPLETION_SURFACE;

#[cfg(test)]
use super::{
    FixedClock, MockConfig, UnusedRegistry, UnusedToolRegistry, block_on, execution_tool_call,
    followup_values, metadata_value, plan_request, test_logger, test_map,
};

#[cfg(test)]
fn run_plan(kwargs: &Map<String, Value>) -> AgenticLoopPlan {
    let config = MockConfig::succeeding(json!({"id": "cnt-1"}), "4");
    let mut logger = test_logger(config);
    let messages = json!([{"role": "user", "content": "hi"}]);
    let optional_params = Map::new();
    let tool_calls = [execution_tool_call(Some("call-1"), "print(1)")];
    let request = plan_request(&messages, &optional_params, &tool_calls, kwargs);
    let clock = FixedClock(Instant::now());
    block_on(build_agentic_loop_plan(
        &mut logger,
        &UnusedRegistry,
        &UnusedToolRegistry,
        &clock,
        &request,
    ))
    .unwrap()
}

#[cfg(test)]
#[test]
fn dispatch_builds_the_chat_plan_only_for_the_chat_surface() {
    let chat_kwargs = test_map([
        (SANDBOX_KEY, json!("k")),
        ("_agentic_loop_api_surface", json!(CHAT_COMPLETION_SURFACE)),
    ]);
    let chat = run_plan(&chat_kwargs);
    assert_eq!(
        metadata_value(&chat.metadata, "response_format"),
        json!("openai")
    );
    let chat_messages = followup_values(&chat.request_patch.unwrap().messages.unwrap());
    assert_eq!(chat_messages[0], json!({"role": "user", "content": "hi"}));
    assert_eq!(chat_messages[1]["role"], json!("assistant"));
    assert_eq!(chat_messages[2]["role"], json!("tool"));
    assert_eq!(chat_messages[2]["tool_call_id"], json!("call-1"));

    let responses_kwargs = test_map([(SANDBOX_KEY, json!("k"))]);
    let responses = run_plan(&responses_kwargs);
    assert!(responses.metadata.get("response_format").is_none());
    let responses_messages = followup_values(&responses.request_patch.unwrap().messages.unwrap());
    assert_eq!(responses_messages[1]["type"], json!("function_call"));
    assert_eq!(responses_messages[2]["type"], json!("function_call_output"));

    let other_kwargs = test_map([
        (SANDBOX_KEY, json!("k")),
        ("_agentic_loop_api_surface", json!("responses")),
    ]);
    let other = run_plan(&other_kwargs);
    assert!(other.metadata.get("response_format").is_none());
    let other_messages = followup_values(&other.request_patch.unwrap().messages.unwrap());
    assert_eq!(other_messages[1]["type"], json!("function_call"));
}
