#![cfg(test)]

use super::shared::{
    Builder, Events, Harness, Kind, capture_logs, chat, describe, logged_text, ok, request_data,
    upstream_failure,
};

#[tokio::test]
async fn a_failed_assembly_flushes_the_buffered_chunks_with_an_error_log() {
    let (captured, _guard) = capture_logs();
    let mut harness = Harness::new();
    harness.assembly.builder = Builder::Fails;
    let mut request = request_data("{}");
    let (sent, result) = harness
        .run(Kind::Mask, &mut request, ok(vec![chat("a"), chat("b")]))
        .await;
    result.unwrap();
    assert_eq!(sent, [describe(&chat("a")), describe(&chat("b"))]);
    assert!(logged_text(&captured).contains("Error masking streaming PII output"));
}

#[tokio::test]
async fn an_upstream_failure_after_buffering_flushes_what_was_buffered() {
    let (captured, _guard) = capture_logs();
    let harness = Harness::new();
    let mut request = request_data("{}");
    let events: Events = vec![Ok(chat("a")), Err(upstream_failure())];
    let (sent, result) = harness.run(Kind::Mask, &mut request, events).await;
    result.unwrap();
    assert_eq!(sent, [describe(&chat("a"))]);
    assert!(logged_text(&captured).contains("Error masking streaming PII output"));
}

#[tokio::test]
async fn an_empty_stream_warns_and_sends_nothing() {
    let (captured, _guard) = capture_logs();
    let harness = Harness::new();
    let mut request = request_data("{}");
    let (sent, result) = harness.run(Kind::Mask, &mut request, Vec::new()).await;
    result.unwrap();
    assert!(sent.is_empty());
    let log = logged_text(&captured);
    assert!(log.contains("contained no ModelResponseStream chunks"));
    assert!(!log.contains("Error masking"));
}

#[tokio::test]
async fn a_stream_that_masks_cleanly_logs_nothing() {
    let (captured, _guard) = capture_logs();
    let harness = Harness::new();
    let mut request = request_data("{}");
    let (_sent, result) = harness
        .run(Kind::Mask, &mut request, ok(vec![chat("a")]))
        .await;
    result.unwrap();
    assert!(logged_text(&captured).is_empty());
}
