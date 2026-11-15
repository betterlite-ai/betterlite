use super::super::*;
use rstest::rstest;

#[rstest]
#[case::at_the_default_trigger(200_000_usize, true)]
#[case::one_token_above_the_default(200_001, false)]
#[tokio::test]
async fn uses_the_two_hundred_thousand_token_default(
    #[case] original_tokens: usize,
    #[case] skipped: bool,
) {
    let messages = vec![json!({"role": "user", "content": "hello"})];
    let counter = fixture_counter(vec![original_tokens]);
    let compress_request = CompressRequest {
        messages: &messages,
        model: MODEL,
        call_type: "completion",
        compression_trigger: DEFAULT_COMPRESSION_TRIGGER,
        compression_target: None,
    };
    let result = compress(&compress_request, &counter)
        .await
        .expect("compressed");
    assert_eq!(result.compression_skipped_reason.is_some(), skipped);
}

#[tokio::test]
async fn passes_messages_through_unchanged() {
    let messages = vec![
        json!({"role": "system", "content": "be helpful"}),
        json!({"role": "user", "content": [{"type": "text", "text": "hello there"}]}),
    ];
    let counter = fixture_counter(vec![10]);
    let result = compress(&request(&messages, 10, None), &counter)
        .await
        .expect("compressed");
    assert_eq!(result.messages, messages);
    assert_eq!(result.original_tokens, 10);
    assert_eq!(result.compressed_tokens, 10);
    assert_eq!(result.compression_ratio.to_bits(), 0.0_f64.to_bits());
    assert!(result.cache.is_empty());
    assert!(result.tools.is_empty());
}
