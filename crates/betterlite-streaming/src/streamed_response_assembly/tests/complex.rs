use std::sync::Arc;

use betterlite_redact::secret::Redactor;

use crate::stream_content_assembly::wire::{AudioResponse, ExpiryValue, ThinkingBlock};
use crate::streamed_response_assembly::records::Chunk;

use super::support::chunks;
use super::{FIXED_NOW, Harness, chat_of, response_of, value};

pub(super) async fn complex_content_order(redactor: &Arc<Redactor>) {
    complex_tool_calls(redactor).await;
    complex_tool_call_text(redactor).await;
    complex_function_calls(redactor).await;
    complex_lists(redactor).await;
    complex_thinking(redactor).await;
    complex_audio(redactor).await;
    complex_created_at_order(redactor).await;
}

const TOOL_CALL_STREAM: &str = r#"[
    {"id": "chatcmpl-1", "object": "chat.completion.chunk", "created": 1700000000, "model": "gpt-4o",
     "choices": [{"index": 0, "delta": {"role": "assistant", "content": "Hi"}}]},
    {"choices": [{"index": 0, "delta": {"tool_calls": [{"index": 0, "id": "call_1", "type": "function",
        "function": {"name": "get_weather", "arguments": "{\"city\":\"Paris\"}"}}]}}]},
    {"choices": [{"index": 0, "delta": {"content": " there"}, "finish_reason": "stop"}]}
]"#;

const TOOL_CALL_TEXT: &str = r#"Hi there{"function":{"arguments":"{\"city\":\"Paris\"}","name":"get_weather"},"id":"call_1","type":"function"}"#;

// Without a usage chunk the counter sees the completion text: the content followed by each tool call's JSON.
pub(super) async fn complex_tool_call_text(redactor: &Arc<Redactor>) {
    let mut harness = Harness::new(Some(chunks(TOOL_CALL_STREAM)), redactor);
    harness.messages = true;
    harness.counter.response_text = 1;
    let response = chat_of(&harness).await;
    assert!(response.choices[0].message.tool_calls.is_some());
    assert_eq!(
        harness.counter.seen_completion().as_deref(),
        Some(TOOL_CALL_TEXT)
    );
}

pub(super) async fn complex_tool_calls(redactor: &Arc<Redactor>) {
    let stream = chunks(
        r#"[
        {"id": "chatcmpl-1", "object": "chat.completion.chunk", "created": 1700000000, "model": "gpt-4o",
         "choices": [{"index": 0, "delta": {"role": "assistant", "content": "Hi"}}]},
        {"choices": [{"index": 0, "delta": {"tool_calls": [{"index": 0, "id": "call_1", "type": "function",
            "function": {"name": "get_weather", "arguments": ""}}]}}]},
        {"choices": [{"index": 0, "delta": {"tool_calls": [{"index": 0,
            "function": {"arguments": "{\"city\":\"Paris\"}"}}]}}]},
        {"choices": [{"index": 0, "delta": {"content": " there"}, "finish_reason": "stop"}],
         "usage": {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15}}
    ]"#,
    );
    let response = chat_of(&Harness::new(Some(stream), redactor)).await;
    let message = &response.choices[0].message;
    assert_eq!(message.content.as_deref(), Some("Hi there"));
    let calls = message
        .tool_calls
        .as_ref()
        .expect("the tool call fragments are assembled");
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].id, "call_1");
    assert_eq!(calls[0].kind, "function");
    let function = calls[0]
        .function
        .as_ref()
        .expect("the assembled call carries its function");
    assert_eq!(function.name.as_deref(), Some("get_weather"));
    assert_eq!(function.arguments, r#"{"city":"Paris"}"#);
    assert!(message.function_call.is_none());
    assert_eq!(response.usage.completion_tokens, 5);
    assert_eq!(response.usage.completion_tokens_details, None);
}

const FUNCTION_CALLS: [(&str, Option<&str>); 2] = [
    (
        r#"[
        {"id": "chatcmpl-1", "object": "chat.completion.chunk", "created": 1700000000, "model": "gpt-4o",
         "choices": [{"index": 0, "delta": {"role": "assistant", "content": "Hi"}}]},
        {"choices": [{"index": 0, "delta": {"function_call": {"name": "get_weather",
            "arguments": "{\"city\":\""}}}]},
        {"choices": [{"index": 0, "delta": {"function_call": {"arguments": "Paris\"}"}}}]},
        {"choices": [{"index": 0, "delta": {"content": " there"}}]}
    ]"#,
        Some("Hi there"),
    ),
    (
        r#"[
        {"id": "chatcmpl-1", "object": "chat.completion.chunk", "created": 1700000000, "model": "gpt-4o",
         "choices": [{"index": 0, "delta": {"role": "assistant", "function_call": {"name": "get_weather",
            "arguments": "{\"city\":\""}}}]},
        {"choices": [{"index": 0, "delta": {"function_call": {"arguments": "Paris\"}"}}}]},
        {"choices": [{"index": 0, "delta": {}, "finish_reason": "function_call"}]}
    ]"#,
        None,
    ),
];

// A function call clears the text, so only a later content chunk brings it back.
pub(super) async fn complex_function_calls(redactor: &Arc<Redactor>) {
    for (source, content) in FUNCTION_CALLS {
        let response = response_of(source, 2, redactor).await;
        let message = &response.choices[0].message;
        assert_eq!(message.content.as_deref(), content);
        assert!(message.tool_calls.is_none());
        let function = message
            .function_call
            .as_ref()
            .expect("the function call fragments are assembled");
        assert_eq!(function.name.as_deref(), Some("get_weather"));
        assert_eq!(function.arguments, r#"{"city":"Paris"}"#);
    }
}

// B2232 sorts by _hidden_params.created_at when the first chunk sets it, so the later arrival joins first.
const CREATED_AT_STREAM: &str = r#"[
    {"id": "chatcmpl-1", "object": "chat.completion.chunk", "created": 1700000000, "model": "gpt-4o",
     "_hidden_params": {"created_at": 2},
     "choices": [{"index": 0, "delta": {"role": "assistant", "content": "B", "reasoning_content": "y"},
        "finish_reason": "stop"}]},
    {"model": "gpt-4o", "_hidden_params": {"created_at": 1},
     "choices": [{"index": 0, "delta": {"content": "A", "reasoning_content": "x"},
        "finish_reason": "length"}]}
]"#;

pub(super) async fn complex_created_at_order(redactor: &Arc<Redactor>) {
    let response = response_of(CREATED_AT_STREAM, 1, redactor).await;
    let choice = &response.choices[0];
    assert_eq!(choice.message.content.as_deref(), Some("AB"));
    assert_eq!(choice.message.reasoning_content.as_deref(), Some("xy"));
    assert_eq!(choice.finish_reason, "stop");
}

const LISTS_STREAM: &str = r#"[
    {"id": "chatcmpl-1", "object": "chat.completion.chunk", "created": 1700000000, "model": "gpt-4o",
     "choices": [{"index": 0, "delta": {"content": "A",
        "annotations": [{"url": "u1"}], "images": [{"url": "img1"}]}}]},
    {"choices": [{"index": 0, "delta": {"annotations": [{"url": "u2"}], "images": []}}]}
]"#;

pub(super) async fn complex_lists(redactor: &Arc<Redactor>) {
    let response = response_of(LISTS_STREAM, 1, redactor).await;
    let message = &response.choices[0].message;
    let annotations = vec![value(r#"{"url": "u1"}"#), value(r#"{"url": "u2"}"#)];
    assert_eq!(message.annotations, Some(annotations));
    assert_eq!(message.images, Some(vec![value(r#"{"url": "img1"}"#)]));
}

pub(super) async fn complex_thinking(redactor: &Arc<Redactor>) {
    let mut harness = Harness::new(Some(thinking_stream()), redactor);
    harness.messages = true;
    harness.counter.text = 2;
    harness.counter.response_text = 3;
    let response = chat_of(&harness).await;
    let message = &response.choices[0].message;
    assert_eq!(message.reasoning_content.as_deref(), Some("thinking"));
    assert_eq!(
        message.thinking_blocks,
        Some(vec![
            ThinkingBlock::Thinking {
                thinking: "deep".to_owned(),
                signature: "sig".to_owned(),
            },
            ThinkingBlock::Redacted {
                data: "d1".to_owned(),
            },
        ])
    );
    assert_eq!(
        harness.counter.seen_reasoning().as_deref(),
        Some("thinking")
    );
    assert_eq!(response.usage.completion_tokens, 5);
    let details = response
        .usage
        .completion_tokens_details
        .expect("the complex path counts reasoning tokens");
    assert_eq!(details.reasoning_tokens, Some(2));
    assert_eq!(details.text_tokens, Some(3));
}

fn thinking_stream() -> Vec<Chunk> {
    chunks(
        r#"[
        {"id": "chatcmpl-1", "object": "chat.completion.chunk", "created": 1700000000, "model": "gpt-4o",
         "choices": [{"index": 0, "delta": {"role": "assistant",
            "thinking_blocks": [{"type": "thinking", "thinking": "deep", "signature": "sig"}],
            "reasoning_content": "think"}}]},
        {"choices": [{"index": 0, "delta": {
            "thinking_blocks": [{"type": "redacted_thinking", "data": "d1"}],
            "reasoning_content": "ing"}}]}
    ]"#,
    )
}

const AUDIO_STREAMS: [&str; 2] = [
    r#"[
    {"id": "chatcmpl-1", "object": "chat.completion.chunk", "created": 1700000000, "model": "gpt-4o",
     "choices": [{"index": 0, "delta": {"content": "A",
        "audio": {"id": "a1", "data": "YQ==", "transcript": "hi"}}}]},
    {"choices": [{"index": 0, "delta": {
        "audio": {"data": "Yg==", "transcript": "!", "expires_at": 1600000000}}}]}
]"#,
    r#"[
    {"id": "chatcmpl-1", "object": "chat.completion.chunk", "created": 1700000000, "model": "gpt-4o",
     "choices": [{"index": 0, "delta": {"content": "A",
        "audio": {"data": "YQ==", "transcript": "hi", "expires_at": false}}}]}
]"#,
];

// The first stream carries its own expiry and id; the second falls back to the clock plus an hour.
pub(super) async fn complex_audio(redactor: &Arc<Redactor>) {
    let explicit = AudioResponse {
        data: "YWI=".to_owned(),
        expires_at: ExpiryValue::Signed(1_600_000_000),
        transcript: "hi!".to_owned(),
        id: Some("a1".to_owned()),
    };
    let defaulted = AudioResponse {
        data: "YQ==".to_owned(),
        expires_at: ExpiryValue::Signed(FIXED_NOW + 3600),
        transcript: "hi".to_owned(),
        id: None,
    };
    for (source, expected) in AUDIO_STREAMS.into_iter().zip([explicit, defaulted]) {
        let response = response_of(source, 1, redactor).await;
        assert_eq!(response.choices[0].message.audio, Some(expected));
    }
}
