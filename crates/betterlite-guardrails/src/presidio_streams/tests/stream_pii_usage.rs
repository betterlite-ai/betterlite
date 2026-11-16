#![cfg(test)]

use super::shared::{Harness, Kind, chat, chat_with_usage, ok, tokens_request};

#[tokio::test]
async fn the_usage_of_the_last_chunk_reaches_the_assembled_chunk() {
    let harness = Harness::new();
    let mut request = tokens_request();
    let events = ok(vec![chat("a"), chat_with_usage("b")]);
    let (sent, result) = harness.run(Kind::Unmask, &mut request, events).await;
    result.unwrap();
    assert_eq!(sent.len(), 1);
    assert!(sent[0].contains(r#""usage":{"total_tokens":7}"#));
}

#[tokio::test]
async fn a_stream_without_usage_stays_without_it() {
    let harness = Harness::new();
    let mut request = tokens_request();
    let (sent, result) = harness
        .run(Kind::Unmask, &mut request, ok(vec![chat("a"), chat("b")]))
        .await;
    result.unwrap();
    assert!(!sent[0].contains("usage"));
}
