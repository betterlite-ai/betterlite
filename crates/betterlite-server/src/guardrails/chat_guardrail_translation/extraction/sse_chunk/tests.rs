#![cfg(test)]

use rstest::rstest;

use crate::guardrails::chat_guardrail_translation::block_framing::sse_chunk::{
    BlockedChunk, BlockedChunkChoice, BlockedChunkDelta, BlockedChunkUsage, chat_sse_chunk,
};

fn chunk<'text>(
    delta: BlockedChunkDelta<'text>,
    finish_reason: Option<&'text str>,
    usage: Option<BlockedChunkUsage>,
) -> BlockedChunk<'text> {
    BlockedChunk {
        id: "chatcmpl-1",
        object: "chat.completion.chunk",
        created: 7,
        model: "gpt",
        choices: [BlockedChunkChoice {
            index: 0,
            delta,
            finish_reason,
        }],
        usage,
    }
}

#[rstest]
#[case::role_and_content(
    BlockedChunkDelta { role: Some("assistant"), content: Some("no") },
    None,
    None,
    r#"data: {"id":"chatcmpl-1","object":"chat.completion.chunk","created":7,"model":"gpt","choices":[{"index":0,"delta":{"role":"assistant","content":"no"},"finish_reason":null}]}"#,
)]
#[case::content_only(
    BlockedChunkDelta { role: None, content: Some("no") },
    None,
    None,
    r#"data: {"id":"chatcmpl-1","object":"chat.completion.chunk","created":7,"model":"gpt","choices":[{"index":0,"delta":{"content":"no"},"finish_reason":null}]}"#,
)]
#[case::final_chunk_with_usage(
    BlockedChunkDelta::default(),
    Some("content_filter"),
    Some(BlockedChunkUsage { prompt_tokens: 3, completion_tokens: 4, total_tokens: 7 }),
    r#"data: {"id":"chatcmpl-1","object":"chat.completion.chunk","created":7,"model":"gpt","choices":[{"index":0,"delta":{},"finish_reason":"content_filter"}],"usage":{"prompt_tokens":3,"completion_tokens":4,"total_tokens":7}}"#,
)]
#[case::message_is_escaped(
    BlockedChunkDelta { role: None, content: Some("say \"hi\"\n") },
    None,
    None,
    r#"data: {"id":"chatcmpl-1","object":"chat.completion.chunk","created":7,"model":"gpt","choices":[{"index":0,"delta":{"content":"say \"hi\"\n"},"finish_reason":null}]}"#,
)]
fn a_chunk_is_one_compact_data_frame(
    #[case] delta: BlockedChunkDelta<'_>,
    #[case] finish_reason: Option<&str>,
    #[case] usage: Option<BlockedChunkUsage>,
    #[case] expected: &str,
) {
    let frame = chat_sse_chunk(&chunk(delta, finish_reason, usage)).unwrap();
    assert_eq!(frame, format!("{expected}\n\n").into_bytes());
}
