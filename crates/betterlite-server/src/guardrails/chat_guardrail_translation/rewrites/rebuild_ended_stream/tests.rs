#![cfg(test)]

use super::{ChunkAssembler, StreamChunkAssembler, rebuild_per_choice};
use crate::guardrails::chat_guardrail_translation::rewrites::error::RewriteError;
use crate::guardrails::chat_guardrail_translation::rewrites::response::{
    ChatResponse, MessageContent,
};
use crate::guardrails::chat_guardrail_translation::rewrites::tests::assemblers::{
    ConcatenatingAssembler, NoCosts, ZeroCounter, hooks,
};
use crate::guardrails::chat_guardrail_translation::rewrites::tests::support::chunks;

const INTERLEAVED: &str = r#"[
    {"choices": [{"index": 0, "delta": {"content": "He"}}]},
    {"choices": [{"index": 1, "delta": {"content": "Ot"}}]},
    {"choices": [{"index": 0, "delta": {"content": "llo"}}]},
    {"choices": [{"index": 1, "delta": {"content": "her"}}]},
    {"usage": {"total_tokens": 7}}]"#;

fn texts(rebuilt: &ChatResponse) -> Vec<(i64, Option<String>)> {
    rebuilt
        .choices
        .iter()
        .map(|choice| match &choice.message.content {
            Some(MessageContent::Text(text)) => (choice.index, Some(text.clone())),
            Some(MessageContent::Parts(_)) | None => (choice.index, None),
        })
        .collect()
}

#[tokio::test]
async fn rebuild_ended_stream_rebuilds_each_choice_alone_from_every_chunk() {
    let assembler = ConcatenatingAssembler::texts();
    let rebuilt = rebuild_per_choice(&chunks(INTERLEAVED), &assembler)
        .await
        .unwrap();
    assert_eq!(
        texts(&rebuilt),
        [(0, Some("Hello".to_owned())), (1, Some("Other".to_owned()))]
    );
    assert_eq!(assembler.chunk_counts(), [5, 5]);
}

#[tokio::test]
async fn rebuild_ended_stream_takes_the_response_fields_from_the_lowest_choice() {
    let assembler = ConcatenatingAssembler::texts();
    let rebuilt = rebuild_per_choice(&chunks(INTERLEAVED), &assembler)
        .await
        .unwrap();
    assert_eq!(rebuilt.model.as_deref(), Some("Hello"));
}

#[tokio::test]
async fn rebuild_ended_stream_keeps_the_index_the_stream_gave_each_choice() {
    let stream = r#"[
        {"choices": [{"index": 5, "delta": {"content": "b"}}]},
        {"choices": [{"index": 2, "delta": {"content": "a"}}]}]"#;
    let assembler = ConcatenatingAssembler::texts();
    let rebuilt = rebuild_per_choice(&chunks(stream), &assembler)
        .await
        .unwrap();
    assert_eq!(
        texts(&rebuilt),
        [(2, Some("a".to_owned())), (5, Some("b".to_owned()))]
    );
}

#[tokio::test]
async fn rebuild_ended_stream_counts_a_choice_without_an_index_as_choice_zero() {
    let stream = r#"[{"choices": [{"delta": {"content": "a"}}]}]"#;
    let assembler = ConcatenatingAssembler::texts();
    let rebuilt = rebuild_per_choice(&chunks(stream), &assembler)
        .await
        .unwrap();
    assert_eq!(texts(&rebuilt), [(0, Some("a".to_owned()))]);
}

#[tokio::test]
async fn rebuild_ended_stream_refuses_a_stream_without_choices() {
    let assembler = ConcatenatingAssembler::texts();
    let failure = rebuild_per_choice(&chunks(r#"[{"usage": {}}]"#), &assembler)
        .await
        .unwrap_err();
    assert!(matches!(failure, RewriteError::NoStreamChoices));
    assert!(assembler.chunk_counts().is_empty());
}

#[tokio::test]
async fn rebuild_ended_stream_refuses_a_rebuilt_response_without_a_choice() {
    let assembler = ConcatenatingAssembler::without_choices();
    let failure = rebuild_per_choice(&chunks(INTERLEAVED), &assembler)
        .await
        .unwrap_err();
    assert!(matches!(
        failure,
        RewriteError::NoRebuiltChoice { index: 0 }
    ));
}

#[tokio::test]
async fn rebuild_ended_stream_assembles_each_choice_with_the_stream_chunk_builder() {
    let stream = r#"[
        {"id": "c1", "object": "chat.completion.chunk", "created": 1700000000, "model": "gpt-4o",
         "choices": [{"index": 0, "delta": {"role": "assistant", "content": "A"}}]},
        {"id": "c1", "object": "chat.completion.chunk", "created": 1700000000, "model": "gpt-4o",
         "choices": [{"index": 1, "delta": {"role": "assistant", "content": "B"}}]},
        {"id": "c1", "object": "chat.completion.chunk", "created": 1700000000, "model": "gpt-4o",
         "choices": [{"index": 0, "delta": {"content": "1"}, "finish_reason": "stop"}]},
        {"id": "c1", "object": "chat.completion.chunk", "created": 1700000000, "model": "gpt-4o",
         "choices": [{"index": 1, "delta": {"content": "2"}, "finish_reason": "stop"}]}]"#;
    let hooks = hooks();
    let assembler = StreamChunkAssembler {
        costs: &NoCosts,
        counter: &ZeroCounter,
        hooks: &hooks,
        now: 1_700_000_000,
    };
    let rebuilt = rebuild_per_choice(&chunks(stream), &assembler)
        .await
        .unwrap();
    assert_eq!(
        texts(&rebuilt),
        [(0, Some("A1".to_owned())), (1, Some("B2".to_owned()))]
    );
    assert_eq!(rebuilt.model.as_deref(), Some("gpt-4o"));
}

#[tokio::test]
async fn rebuild_ended_stream_refuses_a_stream_the_builder_cannot_assemble() {
    let hooks = hooks();
    let assembler = StreamChunkAssembler {
        costs: &NoCosts,
        counter: &ZeroCounter,
        hooks: &hooks,
        now: 1_700_000_000,
    };
    let failure = assembler
        .assemble(&chunks(
            r#"[{"choices": [{"index": 0, "text": "plain completion"}]}]"#,
        ))
        .await
        .unwrap_err();
    assert!(matches!(failure, RewriteError::NotAChatResponse));
    let empty = assembler.assemble(&[]).await.unwrap_err();
    assert!(matches!(empty, RewriteError::NotAChatResponse));
}
