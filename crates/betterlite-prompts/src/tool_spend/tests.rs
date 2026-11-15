#![cfg(test)]

use serde_json::json;

use super::{realtime_tool_call_names, response_tool_call_names};
use crate::response_tool_calls::projections::{ChatToolCall, ToolCallResponse};

fn chat_response(value: serde_json::Value) -> ToolCallResponse {
    serde_json::from_value(value).unwrap()
}

#[test]
fn reads_every_choice_and_keeps_call_order() {
    let response = chat_response(json!({
        "choices": [
            {"message": {"tool_calls": [
                {"id": "call_1", "function": {"name": "search", "arguments": "{}"}},
                {"id": "call_2", "function": {"name": "fetch", "arguments": "{}"}}
            ]}},
            {"message": {"tool_calls": [
                {"id": "call_3", "function": {"name": "read_file", "arguments": "{}"}}
            ]}}
        ]
    }));
    assert_eq!(
        response_tool_call_names(&response),
        ["search", "fetch", "read_file"]
    );
}

#[test]
fn trims_names_and_drops_empty_ones() {
    let response = chat_response(json!({
        "choices": [{"message": {"tool_calls": [
            {"id": "call_1", "function": {"name": "  search  ", "arguments": "{}"}},
            {"id": "call_2", "function": {"name": "   ", "arguments": "{}"}},
            {"id": "call_3", "function": {"name": "", "arguments": "{}"}},
            {"id": "call_4", "function": {"arguments": "{}"}},
            {"id": "call_5", "function": {"name": "fetch", "arguments": "{}"}}
        ]}}]
    }));
    assert_eq!(response_tool_call_names(&response), ["search", "fetch"]);
}

#[test]
fn reads_the_responses_and_anthropic_families() {
    let responses = chat_response(json!({
        "output": [
            {"type": "function_call", "name": "lookup", "call_id": "call_1", "arguments": "{}"},
            {"type": "message", "name": "declared_only"}
        ]
    }));
    assert_eq!(response_tool_call_names(&responses), ["lookup"]);
    let anthropic = chat_response(json!({
        "content": [
            {"type": "tool_use", "id": "toolu_1", "name": "weather", "input": {"city": "Oslo"}}
        ]
    }));
    assert_eq!(response_tool_call_names(&anthropic), ["weather"]);
}

#[test]
fn realtime_calls_read_the_same_projection() {
    let calls = serde_json::from_value::<Vec<ChatToolCall>>(json!([
        {"id": "call_1", "function": {"name": " search ", "arguments": "{}"}},
        {"id": "call_2", "function": {"name": "  ", "arguments": "{}"}},
        {"id": "call_3", "function": {"name": "fetch", "arguments": "{}"}}
    ]))
    .unwrap();
    assert_eq!(realtime_tool_call_names(&calls), ["search", "fetch"]);
    assert!(realtime_tool_call_names(&[]).is_empty());
}
