use super::super::*;

#[tokio::test]
async fn protected_content_can_exceed_the_target() {
    let messages = vec![
        json!({"role": "system", "content": words(20)}),
        json!({"role": "user", "content": "question"}),
    ];
    let counter = fixture_counter(vec![200, 180]);
    let result = compress(&request(&messages, 0, Some(1)), &counter)
        .await
        .expect("compressed");
    assert_eq!(result.messages, messages);
    assert_eq!(result.compressed_tokens, 180);
    assert!(result.compressed_tokens > 1);
}
