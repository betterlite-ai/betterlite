use super::super::*;

#[tokio::test]
async fn at_the_trigger_still_skips() {
    let messages = vec![json!({"role": "user", "content": "one two"})];
    let counter = fixture_counter(vec![10]);
    let result = compress(&request(&messages, 10, None), &counter)
        .await
        .expect("compressed");
    assert_eq!(
        result.compression_skipped_reason,
        Some(CompressionSkippedReason::BelowTrigger)
    );
    assert_eq!(
        CompressionSkippedReason::BelowTrigger.as_str(),
        "below_trigger"
    );
}

#[tokio::test]
async fn above_the_trigger_compresses() {
    let messages = vec![
        json!({"role": "system", "content": "a b"}),
        json!({"role": "user", "content": "c d e f g h"}),
    ];
    let counter = fixture_counter(vec![11, 5]);
    let result = compress(&request(&messages, 10, None), &counter)
        .await
        .expect("compressed");
    assert_eq!(result.compression_skipped_reason, None);
}
