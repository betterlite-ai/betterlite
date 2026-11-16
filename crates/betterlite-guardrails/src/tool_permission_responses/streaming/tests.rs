#![cfg(test)]

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use tokio::sync::mpsc;

use crate::guardrail_base::failure::GuardrailFailure;
use crate::guardrail_message_scope::StreamItem;
use crate::tool_permission_responses::buffer_overflow::StreamBufferLimits;
use crate::tool_permission_responses::error::ResponseToolError;
use crate::tool_permission_responses::tests::support::{
    guardrail, object, rewrite_guardrail, rule, services, skipping_guardrail,
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
        StreamBufferLimits { max_chunks: 2 },
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
        StreamBufferLimits { max_chunks: 1 },
        &mut receiver,
    )
    .await
    .expect_err("the second chunk exceeds the limit");
    assert_eq!(
        raised(error),
        "Streamed response exceeded the 1 chunk buffer limit for tool permission checks, blocking it"
    );
}

#[tokio::test]
async fn post_call_streaming_iterator_hook_blocks_a_stream_past_the_limit() {
    let (sender, receiver) = mpsc::channel(2);
    sender
        .send(StreamItem::Json(GuardrailValue::Null))
        .await
        .expect("send");
    sender
        .send(StreamItem::Json(GuardrailValue::Null))
        .await
        .expect("send");
    drop(sender);
    let (output, mut output_receiver) = mpsc::channel(4);
    let error = post_call_streaming_iterator_hook(
        &guardrail(Vec::new()),
        StreamBufferLimits { max_chunks: 1 },
        &services(),
        receiver,
        output,
    )
    .await
    .expect_err("the stream exceeds the buffer");
    assert!(raised(error).contains("1 chunk buffer limit"));
    assert!(output_receiver.try_recv().is_err());
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

#[tokio::test]
async fn relay_stops_when_the_receiver_is_gone() {
    let (output, receiver) = mpsc::channel(1);
    drop(receiver);
    relay(&output, vec![StreamItem::Json(GuardrailValue::Null)])
        .await
        .expect("a closed output is not an error");
}
