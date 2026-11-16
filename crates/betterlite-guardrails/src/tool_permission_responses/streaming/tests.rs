#![cfg(test)]

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use tokio::sync::mpsc;

use crate::guardrail_base::failure::GuardrailFailure;
use crate::guardrail_message_scope::StreamItem;
use crate::tool_permission::types::{mapping_value, object_list};
use crate::tool_permission_responses::buffer_overflow::StreamBufferLimits;
use crate::tool_permission_responses::error::ResponseToolError;
use crate::tool_permission_responses::tests::support::{
    guardrail, object, rewrite_guardrail, rule, services, skipping_guardrail, text,
};

use super::{buffer_stream, check_assembled_stream, post_call_streaming_iterator_hook, relay};

fn raised(error: ResponseToolError) -> String {
    let ResponseToolError::Guardrail(GuardrailFailure::GuardrailRaised { message, .. }) = error
    else {
        panic!("expected a raised guardrail failure");
    };
    message
}

#[tokio::test]
async fn buffer_stream_accepts_the_chunk_that_fills_the_limit() {
    let (sender, mut receiver) = mpsc::channel(2);
    sender
        .send(StreamItem::Json(GuardrailValue::Null))
        .await
        .expect("send");
    sender
        .send(StreamItem::Json(GuardrailValue::Null))
        .await
        .expect("send");
    drop(sender);
    let chunks = buffer_stream(
        &guardrail(Vec::new()),
        StreamBufferLimits {
            max_chunks: 2,
            max_bytes: 1_000,
        },
        &mut receiver,
    )
    .await
    .expect("the limit is not exceeded");
    assert_eq!(chunks.len(), 2);
}

#[tokio::test]
async fn buffer_stream_rejects_the_chunk_after_the_limit() {
    let (sender, mut receiver) = mpsc::channel(2);
    sender
        .send(StreamItem::Json(GuardrailValue::Null))
        .await
        .expect("send");
    sender
        .send(StreamItem::Json(GuardrailValue::Null))
        .await
        .expect("send");
    drop(sender);
    let error = buffer_stream(
        &guardrail(Vec::new()),
        StreamBufferLimits {
            max_chunks: 1,
            max_bytes: 1_000,
        },
        &mut receiver,
    )
    .await
    .expect_err("the second chunk exceeds the limit");
    assert_eq!(
        raised(error),
        "Streamed response exceeded the 1 chunk or 1000 byte buffer limit for tool permission checks, blocking it"
    );
}

#[tokio::test]
async fn post_call_streaming_iterator_hook_blocks_an_unparseable_raw_stream() {
    let (sender, receiver) = mpsc::channel(2);
    sender
        .send(StreamItem::Bytes(b"data: {not json\n\n".to_vec()))
        .await
        .expect("send");
    drop(sender);
    let (output, mut output_receiver) = mpsc::channel(4);
    let error = post_call_streaming_iterator_hook(
        &guardrail(Vec::new()),
        StreamBufferLimits::default(),
        &services(),
        receiver,
        output,
    )
    .await
    .expect_err("an unparseable raw stream is blocked");
    assert!(raised(error).contains("not a parseable Anthropic SSE stream"));
    assert!(output_receiver.try_recv().is_err());
}

#[tokio::test]
async fn post_call_streaming_iterator_hook_passes_a_non_raw_chunk_through() {
    let (sender, receiver) = mpsc::channel(2);
    sender
        .send(StreamItem::Json(GuardrailValue::Null))
        .await
        .expect("send");
    drop(sender);
    let (output, mut output_receiver) = mpsc::channel(4);
    post_call_streaming_iterator_hook(
        &guardrail(Vec::new()),
        StreamBufferLimits::default(),
        &services(),
        receiver,
        output,
    )
    .await
    .expect("nothing assembles so the chunks pass through");
    assert_eq!(
        output_receiver.recv().await,
        Some(StreamItem::Json(GuardrailValue::Null))
    );
    assert!(output_receiver.recv().await.is_none());
}

#[tokio::test]
async fn post_call_streaming_iterator_hook_relays_an_empty_stream() {
    let (sender, receiver) = mpsc::channel(1);
    drop(sender);
    let (output, mut output_receiver) = mpsc::channel(1);
    post_call_streaming_iterator_hook(
        &guardrail(Vec::new()),
        StreamBufferLimits::default(),
        &services(),
        receiver,
        output,
    )
    .await
    .expect("an empty stream has nothing to check");
    assert!(output_receiver.recv().await.is_none());
}

#[test]
fn check_assembled_stream_runs_without_a_should_run_check() {
    let response = object(
        r#"{"choices":[{"message":{"tool_calls":[{"id":"c1","function":{"name":"shell"}}]}}]}"#,
    );
    let error = check_assembled_stream(&skipping_guardrail(), &response)
        .expect_err("block mode raises even though should_run_guardrail would skip");
    let GuardrailFailure::GuardrailRaised { message, .. } = error else {
        panic!("expected a raised guardrail failure");
    };
    assert_eq!(message, "Tool 'shell' denied by default action");
}

#[test]
fn check_assembled_stream_collects_rewrite_denials() {
    let response = object(
        r#"{"choices":[{"message":{"tool_calls":[{"id":"c1","function":{"name":"shell"}},{"id":"c2","function":{"name":"read"}}]}}]}"#,
    );
    let denied = check_assembled_stream(
        &rewrite_guardrail(vec![rule("block-shell", "deny", "shell")]),
        &response,
    )
    .expect("rewrite mode collects denials");
    assert_eq!(denied.len(), 1);
    assert_eq!(denied[0].call.id, "c1");
}

#[test]
fn check_assembled_stream_returns_nothing_without_tool_calls() {
    let response = object(r#"{"choices":[{"message":{"content":"hi"}}]}"#);
    assert!(
        check_assembled_stream(&guardrail(Vec::new()), &response)
            .expect("nothing to check")
            .is_empty()
    );
}

fn anthropic_tool_stream(tool_name: &str) -> String {
    let frames = [
        r#"{"type":"message_start","message":{"id":"msg_1","type":"message","role":"assistant","model":"claude-test","usage":{"input_tokens":10,"output_tokens":1}}}"#.to_owned(),
        format!(
            r#"{{"type":"content_block_start","index":0,"content_block":{{"type":"tool_use","id":"toolu_1","name":"{tool_name}","input":{{}}}}}}"#
        ),
        r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"cmd\":\"ls\"}"}}"#.to_owned(),
        r#"{"type":"content_block_stop","index":0}"#.to_owned(),
        r#"{"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":5}}"#.to_owned(),
        r#"{"type":"message_stop"}"#.to_owned(),
    ];
    frames
        .iter()
        .map(|data| format!("data: {data}\n\n"))
        .collect()
}

async fn run_anthropic(
    guardrail: &crate::tool_permission::ToolPermissionGuardrail,
    stream: String,
) -> Vec<StreamItem> {
    let (sender, receiver) = mpsc::channel(4);
    sender
        .send(StreamItem::Bytes(stream.into_bytes()))
        .await
        .expect("send");
    drop(sender);
    let (output, mut output_receiver) = mpsc::channel(64);
    post_call_streaming_iterator_hook(
        guardrail,
        StreamBufferLimits::default(),
        &services(),
        receiver,
        output,
    )
    .await
    .expect("the stream checks");
    let mut items = Vec::new();
    while let Some(item) = output_receiver.recv().await {
        items.push(item);
    }
    items
}

fn anthropic_thinking_tool_stream(tool_name: &str) -> String {
    let frames = [
        r#"{"type":"message_start","message":{"id":"msg_1","type":"message","role":"assistant","model":"claude-test","usage":{"input_tokens":10,"output_tokens":1}}}"#.to_owned(),
        r#"{"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":""}}"#.to_owned(),
        r#"{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"hmm"}}"#.to_owned(),
        r#"{"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"sig-abc"}}"#.to_owned(),
        r#"{"type":"content_block_stop","index":0}"#.to_owned(),
        format!(
            r#"{{"type":"content_block_start","index":1,"content_block":{{"type":"tool_use","id":"toolu_1","name":"{tool_name}","input":{{}}}}}}"#
        ),
        r#"{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"cmd\":\"ls\"}"}}"#.to_owned(),
        r#"{"type":"content_block_stop","index":1}"#.to_owned(),
        r#"{"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":5}}"#.to_owned(),
        r#"{"type":"message_stop"}"#.to_owned(),
    ];
    frames
        .iter()
        .map(|data| format!("data: {data}\n\n"))
        .collect()
}

fn emitted_text(items: &[StreamItem]) -> String {
    items
        .iter()
        .filter_map(|item| match item {
            StreamItem::Bytes(bytes) => Some(String::from_utf8_lossy(bytes).into_owned()),
            StreamItem::Json(_) => None,
        })
        .collect()
}

#[tokio::test]
async fn post_call_streaming_iterator_hook_rewrites_a_denied_anthropic_stream() {
    let stream = anthropic_tool_stream("shell");
    let items = run_anthropic(
        &rewrite_guardrail(vec![rule("block-shell", "deny", "shell")]),
        stream,
    )
    .await;
    let text = emitted_text(&items);
    assert!(text.contains(
        "Permission denied: Tool 'shell' denied by rule 'block-shell' (Rule: block-shell)"
    ));
    assert!(text.contains(r#""stop_reason":"end_turn""#));
    assert!(!text.contains(r#""type":"tool_use""#));
}

#[tokio::test]
async fn post_call_streaming_iterator_hook_keeps_thinking_blocks_of_a_denied_anthropic_stream() {
    let stream = anthropic_thinking_tool_stream("shell");
    let items = run_anthropic(
        &rewrite_guardrail(vec![rule("block-shell", "deny", "shell")]),
        stream,
    )
    .await;
    let text = emitted_text(&items);
    assert!(text.contains(r#""type":"thinking""#));
    assert!(text.contains(r#""thinking":"hmm""#));
    assert!(text.contains(r#""signature":"sig-abc""#));
    assert!(text.contains("Permission denied:"));
    assert!(!text.contains(r#""type":"tool_use""#));
}

#[tokio::test]
async fn post_call_streaming_iterator_hook_passes_an_allowed_anthropic_stream_through() {
    let stream = anthropic_tool_stream("read");
    let items = run_anthropic(
        &rewrite_guardrail(vec![rule("block-shell", "deny", "shell")]),
        stream.clone(),
    )
    .await;
    assert_eq!(items, vec![StreamItem::Bytes(stream.into_bytes())]);
}

fn openai_tool_stream() -> Vec<StreamItem> {
    [
        r#"{"id":"chatcmpl-1","model":"gpt-test","choices":[{"index":0,"delta":{"role":"assistant","tool_calls":[{"index":0,"id":"call_1","type":"function","function":{"name":"shell","arguments":""}}]},"finish_reason":null}]}"#,
        r#"{"id":"chatcmpl-1","model":"gpt-test","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{}"}}]},"finish_reason":null}]}"#,
        r#"{"id":"chatcmpl-1","model":"gpt-test","choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}"#,
    ]
    .iter()
    .map(|chunk| StreamItem::Json(GuardrailValue::Object(object(chunk))))
    .collect()
}

async fn run_openai(
    guardrail: &crate::tool_permission::ToolPermissionGuardrail,
    chunks: Vec<StreamItem>,
) -> Vec<StreamItem> {
    let (sender, receiver) = mpsc::channel(chunks.len().max(1));
    for item in chunks {
        sender.send(item).await.expect("send");
    }
    drop(sender);
    let (output, mut output_receiver) = mpsc::channel(8);
    post_call_streaming_iterator_hook(
        guardrail,
        StreamBufferLimits::default(),
        &services(),
        receiver,
        output,
    )
    .await
    .expect("the stream checks");
    let mut items = Vec::new();
    while let Some(item) = output_receiver.recv().await {
        items.push(item);
    }
    items
}

#[tokio::test]
async fn post_call_streaming_iterator_hook_rewrites_a_denied_openai_stream() {
    let items = run_openai(
        &rewrite_guardrail(vec![rule("block-shell", "deny", "shell")]),
        openai_tool_stream(),
    )
    .await;
    let [StreamItem::Json(GuardrailValue::Object(chunk))] = items.as_slice() else {
        panic!("one reconstructed json chunk");
    };
    assert_eq!(chunk.get("id"), Some(&text("chatcmpl-1")));
    assert_eq!(chunk.get("object"), Some(&text("chat.completion.chunk")));
    let choice = chunk
        .get("choices")
        .and_then(object_list)
        .and_then(|choices| choices.first())
        .expect("choice");
    assert_eq!(mapping_value(choice, "finish_reason"), Some(&text("stop")));
    let delta = mapping_value(choice, "delta").expect("delta");
    assert_eq!(
        mapping_value(delta, "tool_calls"),
        Some(&GuardrailValue::Null)
    );
    assert_eq!(
        mapping_value(delta, "content"),
        Some(&text(
            "Permission denied: Tool 'shell' denied by rule 'block-shell' (Rule: block-shell)"
        ))
    );
}

#[tokio::test]
async fn relay_stops_when_the_receiver_is_gone() {
    let (output, receiver) = mpsc::channel(1);
    drop(receiver);
    relay(&output, vec![StreamItem::Json(GuardrailValue::Null)])
        .await
        .expect("a closed output is not an error");
}
