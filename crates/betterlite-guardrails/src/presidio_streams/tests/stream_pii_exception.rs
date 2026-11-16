#![cfg(test)]

use super::shared::{
    Builder, Events, Harness, Kind, capture_logs, chat, describe, logged_text, ok, other,
    tokens_request, upstream_failure,
};
use crate::presidio_streams::error::PresidioStreamError;

#[tokio::test]
async fn a_failed_assembly_logs_an_error_and_yields_the_buffered_chunks() {
    let (captured, _guard) = capture_logs();
    let mut harness = Harness::new();
    harness.assembly.builder = Builder::Fails;
    let mut request = tokens_request();
    let events = ok(vec![chat("a <PERSON_1>"), chat("b")]);
    let (sent, result) = harness.run(Kind::Unmask, &mut request, events).await;
    result.unwrap();
    assert_eq!(
        sent,
        [describe(&chat("a <PERSON_1>")), describe(&chat("b"))]
    );
    assert!(logged_text(&captured).contains("Error in PII streaming processing"));
}

#[tokio::test]
async fn an_upstream_failure_yields_what_was_buffered_and_ends_the_stream() {
    let (captured, _guard) = capture_logs();
    let harness = Harness::new();
    let mut request = tokens_request();
    let events: Events = vec![Ok(chat("a")), Err(upstream_failure())];
    let (sent, result) = harness.run(Kind::Unmask, &mut request, events).await;
    result.unwrap();
    assert_eq!(sent, [describe(&chat("a"))]);
    assert!(logged_text(&captured).contains("Error in PII streaming processing"));
}

#[tokio::test]
async fn a_reader_that_went_away_is_not_logged_as_a_failure() {
    let (captured, _guard) = capture_logs();
    let mut harness = Harness::new();
    harness.reader_gone = true;
    let mut request = tokens_request();
    let events = ok(vec![chat("a"), other("{}")]);
    let (_sent, result) = harness.run(Kind::Unmask, &mut request, events).await;
    assert!(matches!(
        result.unwrap_err(),
        PresidioStreamError::ConsumerClosed
    ));
    assert!(!logged_text(&captured).contains("Error in PII streaming processing"));
}
