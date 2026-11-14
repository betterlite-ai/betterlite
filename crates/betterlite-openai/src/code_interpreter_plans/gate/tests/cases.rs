#[cfg(test)]
use serde_json::json;

#[cfg(test)]
use super::{empty_logger, test_map};

#[cfg(test)]
fn execution_response() -> Value {
    json!({"output": [
        {"type": "message", "content": []},
        {"type": "function_call", "name": "litellm_code_execution", "call_id": "c-1", "arguments": "{\"code\":\"print(1)\"}"},
        {"type": "function_call", "name": "other_tool", "call_id": "c-2", "arguments": "{}"}
    ]})
}

#[cfg(test)]
fn gate_request<'a>(
    response: &'a Value,
    provider: &'a str,
    kwargs: &'a Map<String, Value>,
) -> GateRequest<'a> {
    GateRequest {
        response,
        custom_llm_provider: provider,
        kwargs,
    }
}

#[cfg(test)]
#[test]
fn conditions_deny_a_disabled_logger_or_an_inactive_request() {
    let response = execution_response();
    let active = test_map([(INTERCEPTION_ACTIVE_KEY, json!(true))]);
    let mut logger = empty_logger::<()>();
    assert!(
        should_run_agentic_loop(&logger, &gate_request(&response, "openai", &active)).is_some()
    );
    logger.enabled = false;
    assert!(
        should_run_agentic_loop(&logger, &gate_request(&response, "openai", &active)).is_none()
    );
    let enabled = empty_logger::<()>();
    for marker in [json!(false), json!("true"), json!(null)] {
        let kwargs = test_map([(INTERCEPTION_ACTIVE_KEY, marker)]);
        assert!(
            should_run_agentic_loop(&enabled, &gate_request(&response, "openai", &kwargs))
                .is_none()
        );
    }
    assert!(
        should_run_agentic_loop(&enabled, &gate_request(&response, "openai", &Map::new()))
            .is_none()
    );
}

#[cfg(test)]
#[test]
fn conditions_filter_by_the_enabled_provider_list() {
    let response = execution_response();
    let active = test_map([(INTERCEPTION_ACTIVE_KEY, json!(true))]);
    let mut logger = empty_logger::<()>();
    assert!(should_run_agentic_loop(&logger, &gate_request(&response, "azure", &active)).is_some());
    logger.enabled_providers = Some(vec!["openai".to_owned()]);
    assert!(
        should_run_agentic_loop(&logger, &gate_request(&response, "openai", &active)).is_some()
    );
    assert!(should_run_agentic_loop(&logger, &gate_request(&response, "azure", &active)).is_none());
    logger.enabled_providers = Some(Vec::new());
    assert!(
        should_run_agentic_loop(&logger, &gate_request(&response, "openai", &active)).is_none()
    );
}

#[cfg(test)]
#[test]
fn conditions_deny_a_response_without_code_execution_calls() {
    let active = test_map([(INTERCEPTION_ACTIVE_KEY, json!(true))]);
    let logger = empty_logger::<()>();
    for response in [
        json!({"output": [{"type": "message", "content": []}]}),
        json!({"output": []}),
        json!({}),
    ] {
        assert!(
            should_run_agentic_loop(&logger, &gate_request(&response, "openai", &active)).is_none()
        );
    }
}

#[cfg(test)]
#[test]
fn detection_collects_only_code_execution_function_calls() {
    let response = execution_response();
    let kwargs = test_map([(INTERCEPTION_ACTIVE_KEY, json!(true))]);
    let logger = empty_logger::<()>();
    let calls =
        should_run_agentic_loop(&logger, &gate_request(&response, "openai", &kwargs)).unwrap();
    assert_eq!(calls.tool_calls.len(), 1);
    let tool_call = &calls.tool_calls[0];
    assert_eq!(tool_call.call_id.as_deref(), Some("c-1"));
    assert!(tool_call.id.is_none());
    assert_eq!(tool_call.arguments, "{\"code\":\"print(1)\"}");
}

#[cfg(test)]
#[test]
fn detection_uses_the_chat_extractor_only_on_the_chat_surface() {
    let kwargs = test_map([
        (INTERCEPTION_ACTIVE_KEY, json!(true)),
        (
            "_agentic_loop_api_surface",
            json!(CHAT_COMPLETION_AGENTIC_SURFACE),
        ),
    ]);
    let chat_response = json!({
        "id": "chatcmpl-1",
        "object": "chat.completion",
        "created": 1,
        "model": "gpt-4o",
        "choices": [{"index": 0, "message": {"role": "assistant", "tool_calls": [
            {"id": "call-1", "type": "function", "function": {"name": "litellm_code_execution", "arguments": "{\"code\":\"print(1)\"}"}}
        ]}}]
    });
    let logger = empty_logger::<()>();
    let calls =
        should_run_agentic_loop(&logger, &gate_request(&chat_response, "openai", &kwargs)).unwrap();
    assert_eq!(calls.tool_calls.len(), 1);
    assert_eq!(calls.tool_calls[0].id.as_deref(), Some("call-1"));

    let responses_response = execution_response();
    assert!(
        should_run_agentic_loop(
            &logger,
            &gate_request(&responses_response, "openai", &kwargs)
        )
        .is_none()
    );
    let plain = test_map([(INTERCEPTION_ACTIVE_KEY, json!(true))]);
    assert!(
        should_run_agentic_loop(&logger, &gate_request(&chat_response, "openai", &plain)).is_none()
    );
}
