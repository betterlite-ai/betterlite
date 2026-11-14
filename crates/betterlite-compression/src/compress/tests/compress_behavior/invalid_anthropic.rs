use super::super::*;

fn broken_messages() -> Vec<Value> {
    vec![
        json!({"role": "user", "content": "hi"}),
        json!({"role": "assistant", "content": [
            {"type": "tool_use", "id": "t1", "name": "lookup", "input": {}}
        ]}),
        json!({"role": "user", "content": [{"type": "text", "text": "never mind"}]}),
    ]
}

fn anthropic_request<'request>(messages: &'request [Value]) -> CompressRequest<'request> {
    CompressRequest {
        messages,
        model: MODEL,
        call_type: "anthropic_messages",
        compression_trigger: 0,
        compression_target: None,
    }
}

#[tokio::test]
async fn returns_the_original_messages() {
    let messages = broken_messages();
    let counter = fixture_counter(vec![50]);
    let result = compress(&anthropic_request(&messages), &counter)
        .await
        .expect("compressed");
    assert_eq!(result.messages, messages);
    assert_eq!(result.original_tokens, 50);
    assert_eq!(result.compressed_tokens, 50);
    assert_eq!(result.compression_ratio.to_bits(), 0.0_f64.to_bits());
    assert!(result.cache.is_empty());
    assert!(result.tools.is_empty());
    assert_eq!(
        result.compression_skipped_reason,
        Some(CompressionSkippedReason::InvalidAnthropicToolSequence)
    );
    assert_eq!(
        CompressionSkippedReason::InvalidAnthropicToolSequence.as_str(),
        "invalid_anthropic_tool_sequence"
    );
}

#[tokio::test]
async fn embeddings_are_scored_before_the_sequence_is_checked() {
    let messages = broken_messages();
    let counter = fixture_counter(vec![50]);
    let executor = RecordingExecutor {
        embeddings: vec![
            vec![1.0_f64, 0.0],
            vec![1.0, 0.0],
            vec![0.0, 1.0],
            vec![0.5, 0.5],
        ],
        calls: Mutex::new(Vec::new()),
    };
    let result = compress_with_embeddings(
        &anthropic_request(&messages),
        &counter,
        &EmbeddingScoring {
            model: "text-embedding-3-small",
            parameters: None,
            cache_present: true,
            executor: &executor,
        },
    )
    .await
    .expect("compressed");
    let calls = executor
        .calls
        .lock()
        .expect("recorded embedding calls")
        .clone();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0]["caching"], json!(true));
    assert_eq!(calls[0]["input"].as_array().map(Vec::len), Some(4));
    assert_eq!(
        result.compression_skipped_reason,
        Some(CompressionSkippedReason::InvalidAnthropicToolSequence)
    );
    assert_eq!(result.messages, messages);
}
