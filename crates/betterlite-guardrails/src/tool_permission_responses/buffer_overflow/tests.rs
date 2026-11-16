#![cfg(test)]

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_core::api::api_boundaries::ApiLimits;
use tokio::sync::mpsc;

use crate::guardrail_base::failure::GuardrailFailure;
use crate::guardrail_message_scope::StreamItem;
use crate::tool_permission_responses::error::ResponseToolError;
use crate::tool_permission_responses::streaming::streaming_iterator_hook;
use crate::tool_permission_responses::tests::support::{GUARDRAIL_NAME, guardrail, services};

use super::{DEFAULT_MAX_STREAM_CHUNKS, StreamBufferLimits};

fn raised(error: ResponseToolError) -> GuardrailFailure {
    let ResponseToolError::Guardrail(failure) = error else {
        panic!("expected a guardrail failure");
    };
    failure
}

async fn run_hook(
    limits: StreamBufferLimits,
    items: Vec<StreamItem>,
) -> (Result<(), ResponseToolError>, Vec<StreamItem>) {
    let (sender, receiver) = mpsc::channel(items.len().max(1));
    for item in items {
        sender.send(item).await.expect("send");
    }
    drop(sender);
    let (output, mut output_receiver) = mpsc::channel(8);
    let result = streaming_iterator_hook(
        &guardrail(Vec::new()),
        limits,
        &services(),
        receiver,
        output,
    )
    .await;
    let mut emitted = Vec::new();
    while let Some(item) = output_receiver.recv().await {
        emitted.push(item);
    }
    (result, emitted)
}

#[test]
fn default_stream_buffer_limits_are_ten_thousand_chunks_and_the_json_byte_ceiling() {
    assert_eq!(DEFAULT_MAX_STREAM_CHUNKS, 10_000);
    let limits = StreamBufferLimits::default();
    assert_eq!(limits.max_chunks, 10_000);
    assert_eq!(limits.max_bytes, ApiLimits::default().max_json_bytes);
}

#[test]
fn stream_buffer_limits_keep_a_configured_limit() {
    let limits = StreamBufferLimits {
        max_chunks: 3,
        max_bytes: 9,
    };
    assert_eq!(limits.max_chunks, 3);
    assert_eq!(limits.max_bytes, 9);
}

#[tokio::test]
async fn a_stream_past_the_chunk_limit_raises_a_400_failure_naming_both_limits() {
    let limits = StreamBufferLimits {
        max_chunks: 1,
        max_bytes: 1_000,
    };
    let items = vec![
        StreamItem::Json(GuardrailValue::Null),
        StreamItem::Json(GuardrailValue::Null),
    ];
    let (result, emitted) = run_hook(limits, items).await;
    let error = result.expect_err("the second chunk exceeds the chunk limit");
    let GuardrailFailure::GuardrailRaised {
        guardrail_name,
        message,
        wrap_default_message,
        status_code,
        blocked_content,
    } = raised(error)
    else {
        panic!("expected a raised guardrail failure");
    };
    assert_eq!(guardrail_name.as_deref(), Some(GUARDRAIL_NAME));
    assert_eq!(
        message,
        "Streamed response exceeded the 1 chunk or 1000 byte buffer limit for tool permission checks, blocking it"
    );
    assert!(wrap_default_message);
    assert_eq!(status_code, 400);
    assert!(!blocked_content);
    assert!(emitted.is_empty());
}

#[tokio::test]
async fn a_stream_past_the_byte_limit_raises_a_400_failure_naming_both_limits() {
    let limits = StreamBufferLimits {
        max_chunks: 10,
        max_bytes: 3,
    };
    // A null chunk encodes to four bytes, one past this ceiling.
    let items = vec![StreamItem::Json(GuardrailValue::Null)];
    let (result, emitted) = run_hook(limits, items).await;
    let error = result.expect_err("the chunk is past the byte limit");
    let GuardrailFailure::GuardrailRaised {
        guardrail_name,
        message,
        wrap_default_message,
        status_code,
        blocked_content,
    } = raised(error)
    else {
        panic!("expected a raised guardrail failure");
    };
    assert_eq!(guardrail_name.as_deref(), Some(GUARDRAIL_NAME));
    assert_eq!(
        message,
        "Streamed response exceeded the 10 chunk or 3 byte buffer limit for tool permission checks, blocking it"
    );
    assert!(wrap_default_message);
    assert_eq!(status_code, 400);
    assert!(!blocked_content);
    assert!(emitted.is_empty());
}

#[tokio::test]
async fn a_chunk_that_fills_the_byte_limit_is_relayed() {
    let limits = StreamBufferLimits {
        max_chunks: 10,
        max_bytes: 4,
    };
    let items = vec![StreamItem::Json(GuardrailValue::Null)];
    let (result, emitted) = run_hook(limits, items.clone()).await;
    result.expect("the byte limit is not exceeded");
    assert_eq!(emitted, items);
}
