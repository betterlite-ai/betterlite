use std::str::from_utf8;

use betterlite_openai::stream::chunk::{Chunk, ThinkingKind};
use futures_util::StreamExt as _;
use tokio::time::timeout;

use crate::anthropic_dispatch::AnthropicResult;
use crate::anthropic_dispatch::native::tests::support::wire;
use crate::anthropic_dispatch::tests::gateway::support::{self, Harness, MODEL, TIMEOUT};
use crate::operation::OperationResult;

const MESSAGE_START: &str = r#"{"type":"message_start","message":{"id":"msg_stream","role":"assistant","model":"claude-3-haiku-20240307","usage":{"input_tokens":10,"output_tokens":1}}}"#;
const TOOL_START: &str = r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"tool_1","name":"get_weather","input":{}}}"#;
const TOOL_DELTA: &str = r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"city\":\"Paris\"}"}}"#;
const THINKING_START: &str =
    r#"{"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":""}}"#;
const THINKING_DELTA: &str = r#"{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"hmm"}}"#;
const SIGNATURE_DELTA: &str = r#"{"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"sig-abc"}}"#;
const BLOCK_STOP: &str = r#"{"type":"content_block_stop","index":0}"#;
const TOOL_END: &str =
    r#"{"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":7}}"#;
const TEXT_END: &str = r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"}}"#;
const MESSAGE_STOP: &str = r#"{"type":"message_stop"}"#;

fn sse(events: &[&str]) -> wire::Answer {
    let body = events
        .iter()
        .map(|event| format!("data: {event}\n\n"))
        .collect();
    wire::Answer {
        status: 200,
        content_type: "text/event-stream",
        body,
    }
}

fn request() -> String {
    format!(r#"{{"model":"{MODEL}","stream":true,"messages":[{{"role":"user","content":"hi"}}]}}"#)
}

async fn chunks(events: &[&str]) -> Vec<Chunk> {
    let harness = Harness::start(vec![sse(events)]).await;
    let result = support::complete(&harness, support::stream(request()))
        .await
        .expect("the streaming call answers");
    let OperationResult::Anthropic(AnthropicResult::Stream(frames)) = result else {
        panic!("a streaming call is a frame stream");
    };
    timeout(TIMEOUT, async move {
        let mut frames = frames;
        let mut parsed = Vec::new();
        while let Some(frame) = frames.next().await {
            let frame = frame.expect("a frame");
            let text = from_utf8(&frame).expect("a utf-8 frame");
            let data = text
                .trim_end()
                .strip_prefix("data: ")
                .expect("a data frame");
            if let Some(chunk) =
                Chunk::parse(data.as_bytes(), MODEL, false).expect("a parseable chunk")
            {
                parsed.push(chunk);
            }
        }
        parsed
    })
    .await
    .expect("the frame stream ends")
}

pub(super) async fn stream() {
    let chunks = chunks(&[
        MESSAGE_START,
        TOOL_START,
        TOOL_DELTA,
        BLOCK_STOP,
        TOOL_END,
        MESSAGE_STOP,
    ])
    .await;
    let mut named = false;
    let mut finished = false;
    for chunk in &chunks {
        for choice in &chunk.choices {
            named |= choice.delta.tool_calls.iter().flatten().any(|call| {
                call.id.as_deref() == Some("tool_1")
                    && call
                        .function
                        .as_ref()
                        .is_some_and(|function| function.name.as_deref() == Some("get_weather"))
            });
            finished |= choice.finish_reason.as_deref() == Some("tool_calls");
        }
    }
    assert!(named, "the tool_use block arrives as delta tool_calls");
    assert!(finished, "the stream closes with a finish_reason chunk");
}

pub(super) async fn thinking() {
    let chunks = chunks(&[
        MESSAGE_START,
        THINKING_START,
        THINKING_DELTA,
        SIGNATURE_DELTA,
        BLOCK_STOP,
        TEXT_END,
        MESSAGE_STOP,
    ])
    .await;
    let thought = chunks
        .iter()
        .flat_map(|chunk| &chunk.choices)
        .find(|choice| {
            choice.delta.reasoning_content.as_deref() == Some("hmm")
                && choice.delta.thinking_blocks.as_ref().is_some_and(|blocks| {
                    blocks.len() == 1
                        && blocks[0].kind == ThinkingKind::Thinking
                        && blocks[0].thinking.as_deref() == Some("hmm")
                })
        });
    assert!(thought.is_some(), "the thinking block reaches the delta");
    let signed = chunks
        .iter()
        .flat_map(|chunk| &chunk.choices)
        .any(|choice| {
            choice.delta.thinking_blocks.as_ref().is_some_and(|blocks| {
                blocks
                    .iter()
                    .any(|block| block.signature.as_deref() == Some("sig-abc"))
            })
        });
    assert!(signed, "the signature reaches the same delta");
}
