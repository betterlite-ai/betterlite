use betterlite_openai::chat::MessageContent;
use betterlite_openai::chat::response::ChatUsage;

use crate::oci_chat_wire::cohere::reply::{
    CohereEmitted, handle_cohere_response, handle_cohere_stream_chunk,
};
use crate::oci_chat_wire::cohere::request::{
    adapt_cohere_messages, adapt_cohere_tools, extract_text_content,
};

use super::support::{assert_json, base_response, body, expected_call_id, messages, tools};

#[test]
fn cohere_text_content_joins_text_parts_only() {
    let content: MessageContent = serde_json::from_str(
        r#"[{"type": "text", "text": "a"}, {"type": "image_url", "text": "ignored", "image_url": "x"}, {"type": "text", "text": "b"}]"#,
    )
    .unwrap();
    assert_eq!(extract_text_content(Some(&content)), "ab");
    let text: MessageContent = serde_json::from_str(r#""plain""#).unwrap();
    assert_eq!(extract_text_content(Some(&text)), "plain");
    assert_eq!(extract_text_content(None), "");
}

#[test]
fn cohere_messages_build_history_without_the_last_user_message() {
    let history = adapt_cohere_messages(&messages(
        r#"[
            {"role": "system", "content": "sys"},
            {"role": "user", "content": "first"},
            {"role": "assistant", "content": "answer"},
            {"role": "user", "content": "last"}
        ]"#,
    ));
    assert_json(
        &history,
        r#"[
            {"role": "USER", "message": "first"},
            {"role": "CHATBOT", "message": "answer"}
        ]"#,
    );
}

#[test]
fn cohere_messages_keep_every_message_when_no_user_message_exists() {
    let history =
        adapt_cohere_messages(&messages(r#"[{"role": "assistant", "content": "answer"}]"#));
    assert_json(&history, r#"[{"role": "CHATBOT", "message": "answer"}]"#);
}

#[test]
fn cohere_messages_parse_tool_call_arguments() {
    let history = adapt_cohere_messages(&messages(
        r#"[{"role": "assistant", "content": "", "tool_calls": [
            {"id": "call_1", "type": "function", "function": {"name": "f", "arguments": "{\"b\": 2, \"a\": 1}"}},
            {"id": "call_2", "type": "function", "function": {"name": "g", "arguments": "not json"}},
            {"id": "call_3", "type": "function", "function": {"name": "h", "arguments": "[]"}}
        ]}]"#,
    ));
    assert_json(
        &history,
        r#"[{"role": "CHATBOT", "message": "", "toolCalls": [
            {"name": "f", "parameters": {"b": 2, "a": 1}},
            {"name": "g", "parameters": {}},
            {"name": "h", "parameters": {}}
        ]}]"#,
    );
}

#[test]
fn cohere_messages_merge_consecutive_tool_results_and_look_up_calls() {
    let history = adapt_cohere_messages(&messages(
        r#"[
            {"role": "assistant", "content": "", "tool_calls": [
                {"id": "call_1", "type": "function", "function": {"name": "f", "arguments": "{\"a\": 1}"}}
            ]},
            {"role": "tool", "tool_call_id": "call_1", "content": "one"},
            {"role": "tool", "tool_call_id": "unknown", "content": "two"}
        ]"#,
    ));
    assert_json(
        &history,
        r#"[
            {"role": "CHATBOT", "message": "", "toolCalls": [{"name": "f", "parameters": {"a": 1}}]},
            {"role": "TOOL", "toolResults": [
                {"call": {"name": "f", "parameters": {"a": 1}}, "outputs": [{"output": "one"}]},
                {"call": {"name": "", "parameters": {}}, "outputs": [{"output": "two"}]}
            ]}
        ]"#,
    );
}

#[test]
fn cohere_messages_keep_separated_tool_results_apart() {
    let history = adapt_cohere_messages(&messages(
        r#"[
            {"role": "tool", "tool_call_id": "a", "content": "one"},
            {"role": "user", "content": "middle"},
            {"role": "assistant", "content": "x"},
            {"role": "tool", "tool_call_id": "b", "content": "two"}
        ]"#,
    ));
    let json = serde_json::to_value(&history).unwrap();
    let items = json.as_array().unwrap();
    assert_eq!(items.len(), 3);
    assert_eq!(items[0]["role"], "TOOL");
    assert_eq!(items[1]["role"], "CHATBOT");
    assert_eq!(items[2]["role"], "TOOL");
}

#[test]
fn cohere_messages_look_up_tool_results_across_all_assistant_messages() {
    let history = adapt_cohere_messages(&messages(
        r#"[
            {"role": "assistant", "content": "a", "tool_calls": [
                {"id": "early", "type": "function", "function": {"name": "one", "arguments": "{}"}}
            ]},
            {"role": "assistant", "content": "b", "tool_calls": [
                {"id": "late", "type": "function", "function": {"name": "two", "arguments": "{}"}}
            ]},
            {"role": "tool", "tool_call_id": "early", "content": "result"},
            {"role": "user", "content": "next"}
        ]"#,
    ));
    let json = serde_json::to_value(&history).unwrap();
    assert_eq!(json[2]["toolResults"][0]["call"]["name"], "one");
}

#[test]
fn cohere_messages_look_up_calls_in_assistant_messages_only() {
    let history = adapt_cohere_messages(&messages(
        r#"[
            {"role": "user", "content": "hi", "tool_calls": [
                {"id": "early", "type": "function", "function": {"name": "one", "arguments": "{}"}}
            ]},
            {"role": "tool", "tool_call_id": "early", "content": "result"}
        ]"#,
    ));
    assert_json(
        &history,
        r#"[{"role": "TOOL", "toolResults": [
            {"call": {"name": "", "parameters": {}}, "outputs": [{"output": "result"}]}
        ]}]"#,
    );
}

#[test]
fn cohere_messages_skip_system_developer_and_function_roles() {
    let history = adapt_cohere_messages(&messages(
        r#"[
            {"role": "system", "content": "sys"},
            {"role": "developer", "content": "dev"},
            {"role": "function", "name": "f", "content": "out"}
        ]"#,
    ));
    assert!(history.is_empty());
}

#[test]
fn cohere_tools_map_parameter_definitions() {
    let adapted = adapt_cohere_tools(&tools(
        r#"[{"type": "function", "function": {"name": "f", "description": "desc", "parameters": {
            "type": "object",
            "properties": {
                "s": {"type": "string", "description": "S", "enum": ["a", 1]},
                "i": {"type": "integer", "minimum": 1, "maximum": 9},
                "u": {"type": "weird"},
                "n": {},
                "e": {"type": ""},
                "x": {"type": "string", "description": "X"}
            },
            "required": ["s", "n", "missing", 7]
        }}}]"#,
    ))
    .unwrap();
    assert_json(
        &adapted,
        r#"[{"name": "f", "description": "desc", "parameterDefinitions": {
            "s": {"description": "S. Allowed values: ['a', 1]", "type": "str", "isRequired": true},
            "i": {"description": "Range: min=1, max=9", "type": "int", "isRequired": false},
            "u": {"description": "", "type": "weird", "isRequired": false},
            "n": {"description": "", "type": "str", "isRequired": true},
            "e": {"description": "", "type": "str", "isRequired": false},
            "x": {"description": "X", "type": "str", "isRequired": false}
        }}]"#,
    );
}

#[test]
fn cohere_tools_default_name_description_and_properties() {
    let adapted = adapt_cohere_tools(&tools(
        r#"[{"type": "custom", "custom": {}}, {"type": "function", "function": {"name": "g"}}]"#,
    ))
    .unwrap();
    assert_json(
        &adapted,
        r#"[
            {"name": "", "description": "", "parameterDefinitions": {}},
            {"name": "g", "description": "", "parameterDefinitions": {}}
        ]"#,
    );
}

#[test]
fn cohere_tools_reject_non_object_parameters() {
    let error = adapt_cohere_tools(&tools(
        r#"[{"type": "function", "function": {"name": "f", "parameters": []}}]"#,
    ))
    .unwrap_err();
    assert_eq!(error.status, 400);
    assert_eq!(
        error.message,
        "Tool `function.parameters` must be an object"
    );
}

#[test]
fn cohere_response_fills_text_model_created_and_usage() {
    let mut response = base_response();
    handle_cohere_response(
        &body(
            r#"{"modelId": "cohere.command-r", "modelVersion": "1.0", "chatResponse": {
                "text": "hello", "apiFormat": "COHERE", "finishReason": "COMPLETE",
                "usage": {"promptTokens": 2, "completionTokens": 3, "totalTokens": 5}
            }}"#,
        ),
        "cohere.command-r",
        1_700_000_000,
        200,
        &mut response,
    )
    .unwrap();
    assert_eq!(response.model, "cohere.command-r");
    assert_eq!(response.created, 1_700_000_000);
    let choice = &response.choices[0];
    assert_eq!(choice.index, 0);
    assert_eq!(choice.finish_reason.as_deref(), Some("stop"));
    assert_eq!(choice.message.role, "assistant");
    assert_eq!(choice.message.content.as_deref(), Some("hello"));
    assert_eq!(choice.message.tool_calls, None);
    assert_eq!(response.usage.prompt_tokens, 2);
    assert_eq!(response.usage.completion_tokens, 3);
    assert_eq!(response.usage.total_tokens, 5);
}

#[test]
fn cohere_response_turns_empty_text_into_no_content() {
    let mut response = base_response();
    handle_cohere_response(
        &body(
            r#"{"modelId": "m", "modelVersion": "1", "chatResponse": {"text": "", "apiFormat": "COHERE"}}"#,
        ),
        "m",
        0,
        200,
        &mut response,
    )
    .unwrap();
    assert_eq!(response.choices[0].message.content, None);
    assert_eq!(response.choices[0].finish_reason.as_deref(), Some("stop"));
    assert_eq!(response.usage, ChatUsage::default());
}

fn cohere_tool_call(body_text: &str) -> (String, String) {
    let mut response = base_response();
    handle_cohere_response(&body(body_text), "m", 0, 200, &mut response).unwrap();
    let call = &response.choices[0].message.tool_calls.as_ref().unwrap()[0];
    (call.id.clone(), call.function.arguments.clone())
}

#[test]
fn cohere_response_hashes_nested_parameter_keys_in_a_stable_order() {
    let first = cohere_tool_call(
        r#"{"modelId": "m", "modelVersion": "1", "chatResponse": {
            "text": "", "apiFormat": "COHERE", "toolCalls": [{"name": "f", "parameters": {
                "o": {"b": 1, "a": 2}, "l": [{"d": 4, "c": 3}]
            }}]
        }}"#,
    );
    let second = cohere_tool_call(
        r#"{"modelId": "m", "modelVersion": "1", "chatResponse": {
            "text": "", "apiFormat": "COHERE", "toolCalls": [{"name": "f", "parameters": {
                "l": [{"c": 3, "d": 4}], "o": {"a": 2, "b": 1}
            }}]
        }}"#,
    );
    assert_eq!(
        first.0,
        expected_call_id(
            0,
            "f",
            "{\"l\":[{\"c\":3,\"d\":4}],\"o\":{\"a\":2,\"b\":1}}"
        )
    );
    assert_eq!(first.0, second.0);
    assert_eq!(
        first.1,
        "{\"o\":{\"b\":1,\"a\":2},\"l\":[{\"d\":4,\"c\":3}]}"
    );
    assert_eq!(
        second.1,
        "{\"l\":[{\"c\":3,\"d\":4}],\"o\":{\"a\":2,\"b\":1}}"
    );
}

#[test]
fn cohere_response_keeps_parameters_order_and_hashes_sorted_keys() {
    let mut response = base_response();
    handle_cohere_response(
        &body(
            r#"{"modelId": "m", "modelVersion": "1", "chatResponse": {
                "text": "", "apiFormat": "COHERE", "finishReason": "TOOL_CALL",
                "toolCalls": [{"name": "f", "parameters": {"b": 2, "a": 1}}]
            }}"#,
        ),
        "m",
        0,
        200,
        &mut response,
    )
    .unwrap();
    let tool_calls = response.choices[0].message.tool_calls.as_ref().unwrap();
    assert_eq!(tool_calls[0].function.arguments, "{\"b\":2,\"a\":1}");
    assert_eq!(
        tool_calls[0].id,
        expected_call_id(0, "f", "{\"a\":1,\"b\":2}")
    );
    assert_eq!(
        response.choices[0].finish_reason.as_deref(),
        Some("tool_calls")
    );
}

#[test]
fn cohere_response_reports_a_body_that_does_not_fit_its_type() {
    let mut response = base_response();
    let error = handle_cohere_response(&body("{}"), "m", 0, 401, &mut response).unwrap_err();
    assert_eq!(error.status, 401);
    assert!(
        error
            .message
            .starts_with("Response cannot be casted to CohereChatResult:")
    );
}

#[test]
fn cohere_stream_emits_text_and_defaults_the_index() {
    let choice = handle_cohere_stream_chunk(
        &body(r#"{"apiFormat": "COHERE", "text": "hi", "finishReason": "COMPLETE"}"#),
        CohereEmitted::default(),
    )
    .unwrap();
    assert_eq!(choice.index, 0);
    assert_eq!(choice.delta.content.as_deref(), Some("hi"));
    assert_eq!(choice.finish_reason.as_deref(), Some("stop"));
}

#[test]
fn cohere_stream_drops_restated_text_after_prior_text() {
    let choice = handle_cohere_stream_chunk(
        &body(
            r#"{"apiFormat": "COHERE", "text": "again", "chatHistory": [{"role": "USER", "message": "x"}]}"#,
        ),
        CohereEmitted {
            tool_calls: false,
            text: true,
        },
    )
    .unwrap();
    assert_eq!(choice.delta.content, None);
}

#[test]
fn cohere_stream_drops_text_restated_by_a_tool_call_chunk() {
    let choice = handle_cohere_stream_chunk(
        &body(
            r#"{"apiFormat": "COHERE", "text": "again", "toolCalls": [{"name": "f", "parameters": {}}]}"#,
        ),
        CohereEmitted {
            tool_calls: false,
            text: true,
        },
    )
    .unwrap();
    assert_eq!(choice.delta.content, None);
}

#[test]
fn cohere_stream_keeps_restated_text_without_prior_text() {
    let choice = handle_cohere_stream_chunk(
        &body(
            r#"{"apiFormat": "COHERE", "text": "only", "chatHistory": [{"role": "USER", "message": "x"}]}"#,
        ),
        CohereEmitted::default(),
    )
    .unwrap();
    assert_eq!(choice.delta.content.as_deref(), Some("only"));
}

#[test]
fn cohere_stream_drops_restated_tool_calls_after_prior_calls() {
    let choice = handle_cohere_stream_chunk(
        &body(
            r#"{"apiFormat": "COHERE", "chatHistory": [{"role": "USER", "message": "x"}],
                "toolCalls": [{"name": "f", "parameters": {"a": 1}}]}"#,
        ),
        CohereEmitted {
            tool_calls: true,
            text: false,
        },
    )
    .unwrap();
    assert!(choice.delta.tool_calls.is_none());
}

#[test]
fn cohere_stream_keeps_tool_calls_that_are_not_restated() {
    let choice = handle_cohere_stream_chunk(
        &body(r#"{"apiFormat": "COHERE", "toolCalls": [{"name": "f", "parameters": {"a": 1}}]}"#),
        CohereEmitted {
            tool_calls: true,
            text: false,
        },
    )
    .unwrap();
    let tool_calls = choice.delta.tool_calls.as_ref().unwrap();
    assert_eq!(tool_calls[0].index, 0);
    assert_eq!(tool_calls[0].kind.as_deref(), Some("function"));
    assert_eq!(
        tool_calls[0].function.as_ref().unwrap().arguments,
        "{\"a\":1}"
    );
    assert_eq!(
        tool_calls[0].id.as_deref(),
        Some(expected_call_id(0, "f", "{\"a\":1}").as_str())
    );
}

#[test]
fn cohere_stream_keeps_tool_calls_without_prior_calls() {
    let choice = handle_cohere_stream_chunk(
        &body(
            r#"{"apiFormat": "COHERE", "chatHistory": [{"role": "USER", "message": "x"}],
                "toolCalls": [{"name": "f", "parameters": {}}]}"#,
        ),
        CohereEmitted::default(),
    )
    .unwrap();
    assert!(choice.delta.tool_calls.is_some());
}

#[test]
fn cohere_stream_ignores_an_empty_tool_call_list() {
    let choice = handle_cohere_stream_chunk(
        &body(r#"{"apiFormat": "COHERE", "toolCalls": []}"#),
        CohereEmitted::default(),
    )
    .unwrap();
    assert!(choice.delta.tool_calls.is_none());
}

#[test]
fn cohere_response_keeps_no_tool_calls_for_an_empty_list() {
    let mut response = base_response();
    handle_cohere_response(
        &body(
            r#"{"modelId": "m", "modelVersion": "1", "chatResponse": {
                "text": "hi", "apiFormat": "COHERE", "toolCalls": []
            }}"#,
        ),
        "m",
        0,
        200,
        &mut response,
    )
    .unwrap();
    assert!(response.choices[0].message.tool_calls.is_none());
    assert_eq!(response.choices[0].message.content.as_deref(), Some("hi"));
}

#[test]
fn cohere_stream_normalizes_an_unknown_finish_reason_to_stop() {
    let choice = handle_cohere_stream_chunk(
        &body(r#"{"apiFormat": "COHERE", "finishReason": "USER_CANCEL"}"#),
        CohereEmitted::default(),
    )
    .unwrap();
    assert_eq!(choice.finish_reason.as_deref(), Some("stop"));
}

#[test]
fn cohere_stream_keeps_an_explicit_index() {
    let choice = handle_cohere_stream_chunk(
        &body(r#"{"apiFormat": "COHERE", "index": 4}"#),
        CohereEmitted::default(),
    )
    .unwrap();
    assert_eq!(choice.index, 4);
}

#[test]
fn cohere_stream_rejects_an_unknown_history_role() {
    let error = handle_cohere_stream_chunk(
        &body(r#"{"apiFormat": "COHERE", "chatHistory": [{"role": "BOT", "message": "x"}]}"#),
        CohereEmitted::default(),
    )
    .unwrap_err();
    assert_eq!(error.status, 500);
    assert!(
        error
            .message
            .starts_with("Chunk cannot be parsed as CohereStreamChunk:")
    );
}

#[test]
fn cohere_stream_reports_a_chunk_that_does_not_fit_its_type() {
    let error = handle_cohere_stream_chunk(&body(r#"{"text": "x"}"#), CohereEmitted::default())
        .unwrap_err();
    assert_eq!(error.status, 500);
    assert!(
        error
            .message
            .starts_with("Chunk cannot be parsed as CohereStreamChunk:")
    );
}
