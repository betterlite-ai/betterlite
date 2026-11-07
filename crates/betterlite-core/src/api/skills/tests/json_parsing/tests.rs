#![cfg(test)]

use betterlite_openai::chat::response::ChatResponse;

use crate::api::skills::hook::response::{GeneratedFilesAttribute, HookResponse, ObjectResponse};
use crate::api::skills::hook::tool_calls::{ExtractedToolCall, ToolCallError, extract_tool_calls};

fn response_with_arguments(arguments: &str) -> HookResponse {
    let escaped = serde_json::to_string(arguments).unwrap();
    let body = format!(
        r#"{{"id":"chatcmpl-1","object":"chat.completion","created":1,"model":"gpt-4o","choices":[{{"index":0,"message":{{"role":"assistant","content":null,"tool_calls":[{{"id":"call_1","type":"function","function":{{"name":"litellm_code_execution","arguments":{escaped}}}}}]}},"finish_reason":"tool_calls"}}]}}"#
    );
    let parsed: ChatResponse = serde_json::from_str(&body).unwrap();
    HookResponse::Object(ObjectResponse {
        content: Vec::new(),
        choices: parsed.choices,
        generated_files: GeneratedFilesAttribute::Rejected,
        model_extra: None,
    })
}

fn expected_tool_call(input: &str) -> ExtractedToolCall {
    ExtractedToolCall {
        id: Some("call_1".to_owned()),
        name: Some("litellm_code_execution".to_owned()),
        input: serde_json::from_str(input).unwrap(),
    }
}

#[test]
fn json_parsing_malformed_arguments_propagate_as_a_parse_error() {
    let error = extract_tool_calls(&response_with_arguments("{not json")).unwrap_err();
    let ToolCallError::Arguments { name, .. } = error;
    assert_eq!(name, "litellm_code_execution");
}

#[test]
fn json_parsing_whitespace_only_arguments_are_malformed() {
    let error = extract_tool_calls(&response_with_arguments("   ")).unwrap_err();
    assert!(matches!(error, ToolCallError::Arguments { .. }));
}

#[test]
fn json_parsing_valid_arguments_keep_their_json_shape() {
    assert_eq!(
        extract_tool_calls(&response_with_arguments(
            r#"{"code": "print(1)", "files": ["a"]}"#
        ))
        .unwrap(),
        vec![expected_tool_call(
            r#"{"code": "print(1)", "files": ["a"]}"#
        )]
    );
}

#[test]
fn json_parsing_accepts_a_scalar_document_like_json_loads() {
    assert_eq!(
        extract_tool_calls(&response_with_arguments("42")).unwrap(),
        vec![expected_tool_call("42")]
    );
}
