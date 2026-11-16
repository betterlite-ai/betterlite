#![cfg(test)]

use super::anthropic_sse_chunks_from_response;
use crate::guardrails::anthropic_sse::error::AnthropicSseError;
use crate::guardrails::anthropic_sse::tests::support::record;

const TEXT_RESPONSE: &str = r#"{"id": "chatcmpl-1", "model": "gpt-4o", "choices": [{"message": {"content": "hello"}, "finish_reason": "stop"}], "usage": {"prompt_tokens": 3, "completion_tokens": 2}}"#;

#[test]
fn anthropic_sse_chunks_from_response_replays_a_message_as_frames() {
    let frames = anthropic_sse_chunks_from_response(&record(TEXT_RESPONSE)).unwrap();
    let events: Vec<String> = frames
        .iter()
        .map(|frame| {
            let text = String::from_utf8(frame.clone()).unwrap();
            text.lines().next().unwrap().to_owned()
        })
        .collect();
    assert_eq!(
        events,
        [
            "event: message_start",
            "event: content_block_start",
            "event: content_block_delta",
            "event: content_block_stop",
            "event: message_delta",
            "event: message_stop",
        ]
    );
}

#[test]
fn anthropic_sse_chunks_from_response_carries_the_id_model_and_text() {
    let frames = anthropic_sse_chunks_from_response(&record(TEXT_RESPONSE)).unwrap();
    assert_eq!(frames.len(), 6);
    let start = String::from_utf8(frames[0].clone()).unwrap();
    let delta = String::from_utf8(frames[2].clone()).unwrap();
    assert!(start.contains("\"id\": \"chatcmpl-1\""));
    assert!(start.contains("\"model\": \"gpt-4o\""));
    assert!(delta.contains("\"text\": \"hello\""));
}

#[test]
fn anthropic_sse_chunks_from_response_fails_on_tool_arguments_that_are_not_an_object() {
    let response = record(
        r#"{"id": "x", "choices": [{"message": {"tool_calls": [{"id": "t", "function": {"name": "f", "arguments": "[1]"}}]}}]}"#,
    );
    let failure = anthropic_sse_chunks_from_response(&response).unwrap_err();
    assert!(matches!(failure, AnthropicSseError::Output(_)));
}
