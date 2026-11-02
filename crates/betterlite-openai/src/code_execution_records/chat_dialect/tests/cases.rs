#[cfg(test)]
use serde_json::json;

#[cfg(test)]
use super::test_object;

#[cfg(test)]
fn sample_response(tool_calls: Value) -> Value {
    test_object([
        ("id", json!("chatcmpl-1")),
        ("object", json!("chat.completion")),
        ("created", json!(1_i64)),
        ("model", json!("gpt-5")),
        (
            "choices",
            Value::Array(vec![test_object([
                ("index", json!(0_i64)),
                (
                    "message",
                    test_object([("role", json!("assistant")), ("tool_calls", tool_calls)]),
                ),
                ("finish_reason", json!("tool_calls")),
            ])]),
        ),
    ])
}

#[cfg(test)]
#[test]
fn to_model_response_builds_a_chat_response_from_a_matching_dict() {
    let response = sample_response(json!(null));
    let model_response = to_model_response(&response).unwrap();
    assert_eq!(model_response.id, "chatcmpl-1");
}

#[cfg(test)]
#[test]
fn to_model_response_returns_none_for_a_non_dict_value() {
    assert_eq!(to_model_response(&json!("not a response")), None);
}

#[cfg(test)]
#[test]
fn to_model_response_returns_none_when_required_fields_are_missing() {
    assert_eq!(
        to_model_response(&test_object([("id", json!("chatcmpl-1"))])),
        None
    );
}

#[cfg(test)]
fn code_execution_tool_call(id: &str, arguments: &str) -> ToolCall {
    ToolCall {
        id: id.to_owned(),
        kind: "function".to_owned(),
        function: FunctionCall {
            name: CODE_EXECUTION_TOOL_NAME.to_owned(),
            arguments: arguments.to_owned(),
        },
    }
}

#[cfg(test)]
#[test]
fn normalize_tool_call_matches_litellm_code_execution_function_calls() {
    let tool_call = code_execution_tool_call("call_1", "{\"code\": \"1\"}");
    let normalized = normalize_tool_call(&tool_call).unwrap();
    assert_eq!(normalized.id, Some("call_1".to_owned()));
    assert_eq!(normalized.call_id, Some("call_1".to_owned()));
    assert_eq!(normalized.arguments, "{\"code\": \"1\"}");
}

#[cfg(test)]
#[test]
fn normalize_tool_call_returns_none_for_a_different_function_name() {
    let mut tool_call = code_execution_tool_call("call_1", "{}");
    tool_call.function.name = "web_search".to_owned();
    assert_eq!(normalize_tool_call(&tool_call), None);
}

#[cfg(test)]
#[test]
fn extract_tool_calls_normalizes_the_first_choices_matching_calls() {
    let tool_calls = Value::Array(vec![
        test_object([
            ("id", json!("call_1")),
            ("type", json!("function")),
            (
                "function",
                test_object([
                    ("name", json!(CODE_EXECUTION_TOOL_NAME)),
                    ("arguments", json!("{}")),
                ]),
            ),
        ]),
        test_object([
            ("id", json!("call_2")),
            ("type", json!("function")),
            (
                "function",
                test_object([("name", json!("web_search")), ("arguments", json!("{}"))]),
            ),
        ]),
    ]);
    let response = sample_response(tool_calls);
    let extracted = extract_tool_calls(&response);
    assert_eq!(extracted.len(), 1);
    assert_eq!(extracted[0].id, Some("call_1".to_owned()));
}

#[cfg(test)]
#[test]
fn extract_tool_calls_returns_empty_when_the_response_cannot_be_parsed() {
    assert_eq!(extract_tool_calls(&json!("not a response")), Vec::new());
}

#[cfg(test)]
#[test]
fn extract_tool_calls_returns_empty_when_the_message_has_no_tool_calls() {
    let response = sample_response(json!(null));
    assert_eq!(extract_tool_calls(&response), Vec::new());
}

#[cfg(test)]
#[test]
fn build_assistant_message_nests_name_and_arguments_under_function() {
    let tool_calls = vec![CodeExecutionToolCall {
        id: Some("call_1".to_owned()),
        call_id: None,
        kind: Some("function".to_owned()),
        name: "ignored".to_owned(),
        arguments: "{\"code\": \"1\"}".to_owned(),
    }];
    let message = build_assistant_message(&tool_calls);
    assert_eq!(message.role, Role::Assistant);
    let built = &message.tool_calls.unwrap()[0];
    assert_eq!(built.id, "call_1");
    assert_eq!(built.function.name, CODE_EXECUTION_TOOL_NAME);
    assert_eq!(built.function.arguments, "{\"code\": \"1\"}");
}

#[cfg(test)]
#[test]
fn build_assistant_message_falls_back_to_call_id_when_id_is_absent() {
    let tool_calls = vec![CodeExecutionToolCall {
        id: None,
        call_id: Some("resp_1".to_owned()),
        kind: None,
        name: "ignored".to_owned(),
        arguments: "{}".to_owned(),
    }];
    let message = build_assistant_message(&tool_calls);
    assert_eq!(message.tool_calls.unwrap()[0].id, "resp_1");
}

#[cfg(test)]
#[test]
fn build_assistant_message_serializes_with_only_role_and_tool_calls() {
    let tool_calls = vec![CodeExecutionToolCall {
        id: Some("call_1".to_owned()),
        call_id: None,
        kind: Some("function".to_owned()),
        name: "ignored".to_owned(),
        arguments: "{}".to_owned(),
    }];
    let message = build_assistant_message(&tool_calls);
    let serialized = serde_json::to_value(&message).unwrap();
    let keys: Vec<&String> = serialized.as_object().unwrap().keys().collect();
    assert_eq!(keys, vec!["role", "tool_calls"]);
}
