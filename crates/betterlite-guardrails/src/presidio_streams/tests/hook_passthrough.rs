#![cfg(test)]

use super::shared::{
    Events, Harness, Kind, bytes, chat, describe, ok, other, request_data, upstream_failure,
};
use crate::presidio_streams::error::PresidioStreamError;

#[tokio::test]
async fn every_kind_of_event_is_relayed_unchanged_in_order() {
    let harness = Harness::new();
    let mut request = request_data("{}");
    let events = ok(vec![
        chat("a"),
        bytes("data: x\n\n"),
        other(r#"{"type":"t"}"#),
    ]);
    let (sent, result) = harness.run(Kind::Hook, &mut request, events).await;
    result.unwrap();
    assert_eq!(
        sent,
        [
            describe(&chat("a")),
            "bytes data: x\n\n".to_owned(),
            describe(&other(r#"{"type":"t"}"#)),
        ]
    );
    assert!(harness.assembly.seen_messages.lock().unwrap().is_empty());
}

#[tokio::test]
async fn an_upstream_failure_is_raised_after_the_events_before_it() {
    let harness = Harness::new();
    let mut request = request_data("{}");
    let events: Events = vec![Ok(chat("a")), Err(upstream_failure())];
    let (sent, result) = harness.run(Kind::Hook, &mut request, events).await;
    assert_eq!(sent, [describe(&chat("a"))]);
    assert!(matches!(
        result.unwrap_err(),
        PresidioStreamError::Upstream { .. }
    ));
}

#[tokio::test]
async fn a_reader_that_went_away_ends_the_relay_without_an_error() {
    let mut harness = Harness::new();
    harness.reader_gone = true;
    let mut request = request_data("{}");
    let (sent, result) = harness
        .run(Kind::Hook, &mut request, ok(vec![chat("a")]))
        .await;
    result.unwrap();
    assert!(sent.is_empty());
}

#[tokio::test]
async fn malformed_pii_tokens_fail_the_hook() {
    let harness = Harness::new();
    let mut request = request_data(r#"{"metadata":{"pii_tokens":[1]}}"#);
    let (sent, result) = harness
        .run(Kind::Hook, &mut request, ok(vec![chat("a")]))
        .await;
    assert!(matches!(
        result.unwrap_err(),
        PresidioStreamError::Actions(_)
    ));
    assert!(sent.is_empty());
}
