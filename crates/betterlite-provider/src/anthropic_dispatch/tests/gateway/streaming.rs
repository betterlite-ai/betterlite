use std::fmt::Write as _;
use std::str::from_utf8;

use betterlite_openai::stream::chunk::{Choice, Chunk, ThinkingBlock, ThinkingKind};
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

const TOOL_EVENTS: [&str; 6] = [
    MESSAGE_START,
    TOOL_START,
    TOOL_DELTA,
    BLOCK_STOP,
    TOOL_END,
    MESSAGE_STOP,
];
const THINKING_EVENTS: [&str; 7] = [
    MESSAGE_START,
    THINKING_START,
    THINKING_DELTA,
    SIGNATURE_DELTA,
    BLOCK_STOP,
    TEXT_END,
    MESSAGE_STOP,
];

fn sse(events: &[&str]) -> wire::Answer {
    let body = events.iter().fold(String::new(), |mut body, event| {
        write!(body, "data: {event}\n\n").expect("the body is a string");
        body
    });
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
    let result = support::complete(&harness, support::stream(&request()))
        .await
        .expect("the streaming call answers");
    let OperationResult::Anthropic(AnthropicResult::Stream(frames)) = result else {
        panic!("a streaming call is a frame stream");
    };
    timeout(TIMEOUT, async move {
        let mut stream = frames;
        let mut parsed = Vec::new();
        while let Some(frame) = stream.next().await {
            let received = frame.expect("a frame");
            let text = from_utf8(&received).expect("a utf-8 frame");
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

fn choices(chunks: &[Chunk]) -> impl Iterator<Item = &Choice> + '_ {
    chunks.iter().flat_map(|chunk| &chunk.choices)
}

fn tool_calls(chunks: &[Chunk]) -> Vec<(&str, &str)> {
    choices(chunks)
        .flat_map(|choice| choice.delta.tool_calls.iter().flatten())
        .filter_map(|call| {
            Some((
                call.id.as_deref()?,
                call.function.as_ref()?.name.as_deref()?,
            ))
        })
        .collect()
}

fn thinking_deltas(chunks: &[Chunk]) -> (Vec<&str>, Vec<&ThinkingBlock>) {
    // The signature-only chunk repeats the reasoning field empty, so only real text counts.
    let reasoning = choices(chunks)
        .filter_map(|choice| choice.delta.reasoning_content.as_deref())
        .filter(|text| !text.is_empty())
        .collect();
    let blocks = choices(chunks)
        .flat_map(|choice| choice.delta.thinking_blocks.iter().flatten())
        .collect();
    (reasoning, blocks)
}

pub(super) async fn stream() {
    let parsed = chunks(&TOOL_EVENTS).await;
    let calls = tool_calls(&parsed);
    assert_eq!(calls, vec![("tool_1", "get_weather")]);
    assert!(
        choices(&parsed).any(|choice| choice.finish_reason.as_deref() == Some("tool_calls")),
        "the stream closes with a finish_reason chunk"
    );
}

pub(super) async fn thinking() {
    let parsed = chunks(&THINKING_EVENTS).await;
    let (reasoning, blocks) = thinking_deltas(&parsed);
    assert_eq!(
        reasoning,
        vec!["hmm"],
        "the reasoning text reaches the delta"
    );
    let thought = blocks.iter().any(|block| {
        block.kind == ThinkingKind::Thinking && block.thinking.as_deref() == Some("hmm")
    });
    assert!(thought, "the thinking block reaches the delta");
    let signed = blocks
        .iter()
        .any(|block| block.signature.as_deref() == Some("sig-abc"));
    assert!(signed, "the signature reaches the same delta");
}
