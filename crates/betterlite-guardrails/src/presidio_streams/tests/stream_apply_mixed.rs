#![cfg(test)]

use super::shared::{
    Harness, Kind, capture_logs, chat, describe, logged_text, ok, other, request_data,
};

const EVENT: &str = r#"{"type":"response.created"}"#;

#[tokio::test]
async fn an_unknown_event_flushes_the_buffer_and_switches_to_passthrough() {
    let (captured, _guard) = capture_logs();
    let harness = Harness::new();
    let mut request = request_data("{}");
    let events = ok(vec![chat("a"), chat("b"), other(EVENT), chat("c")]);
    let (sent, result) = harness.run(Kind::Mask, &mut request, events).await;
    result.unwrap();
    assert_eq!(
        sent,
        [
            describe(&chat("a")),
            describe(&chat("b")),
            describe(&other(EVENT)),
            describe(&chat("c")),
        ]
    );
    let log = logged_text(&captured);
    assert!(log.contains("Flushing 2 buffered chunks"));
    assert!(log.contains("Output PII masking was skipped"));
    assert!(harness.assembly.seen_messages.lock().unwrap().is_empty());
}

#[tokio::test]
async fn an_unknown_event_first_warns_once_without_a_flush() {
    let (captured, _guard) = capture_logs();
    let harness = Harness::new();
    let mut request = request_data("{}");
    let (sent, result) = harness
        .run(Kind::Mask, &mut request, ok(vec![other(EVENT), chat("a")]))
        .await;
    result.unwrap();
    assert_eq!(sent, [describe(&other(EVENT)), describe(&chat("a"))]);
    let log = logged_text(&captured);
    assert!(!log.contains("mixed stream detected"));
    assert!(log.contains("not a parsed chat completion stream"));
}

#[tokio::test]
async fn chunks_after_the_flush_are_not_buffered_again() {
    let harness = Harness::new();
    let mut request = request_data("{}");
    let events = ok(vec![chat("a"), other(EVENT), chat("b"), chat("c")]);
    let (sent, result) = harness.run(Kind::Mask, &mut request, events).await;
    result.unwrap();
    assert_eq!(sent.len(), 4);
    assert_eq!(sent.last(), Some(&describe(&chat("c"))));
}
