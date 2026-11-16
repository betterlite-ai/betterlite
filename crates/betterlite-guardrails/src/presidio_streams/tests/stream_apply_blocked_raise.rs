#![cfg(test)]

use betterlite_config::sections::guardrail_identity::enums::PiiAction;

use super::shared::{
    Events, Harness, Kind, capture_logs, chat, logged_text, ok, request_data,
    spawn_person_analyzer, upstream_failure,
};
use crate::presidio_streams::error::PresidioStreamError;

#[tokio::test]
async fn a_blocked_entity_is_raised_without_yielding_the_chunks() {
    let (base, _server) = spawn_person_analyzer().await;
    let mut harness = Harness::with_base(&base, None);
    harness
        .entities
        .insert("PERSON".to_owned(), PiiAction::Block);
    let mut request = request_data("{}");
    let (sent, result) = harness
        .run(Kind::Mask, &mut request, ok(vec![chat("Alice"), chat("x")]))
        .await;
    let error = result.unwrap_err();
    assert!(error.is_blocked());
    assert_eq!(error.status(), 400);
    assert!(sent.is_empty());
}

#[tokio::test]
async fn an_upstream_failure_with_nothing_buffered_is_raised() {
    let harness = Harness::new();
    let mut request = request_data("{}");
    let events: Events = vec![Err(upstream_failure())];
    let (sent, result) = harness.run(Kind::Mask, &mut request, events).await;
    assert!(matches!(
        result.unwrap_err(),
        PresidioStreamError::Upstream { .. }
    ));
    assert!(sent.is_empty());
}

#[tokio::test]
async fn a_reader_that_went_away_is_never_swallowed() {
    let (captured, _guard) = capture_logs();
    let mut harness = Harness::new();
    harness.reader_gone = true;
    let mut request = request_data("{}");
    let events = ok(vec![chat("a"), super::shared::other("{}")]);
    let (_sent, result) = harness.run(Kind::Mask, &mut request, events).await;
    assert!(matches!(
        result.unwrap_err(),
        PresidioStreamError::ConsumerClosed
    ));
    assert!(!logged_text(&captured).contains("Error masking"));
}

#[test]
fn the_error_statuses_follow_the_wrapped_failure() {
    let blocked = PresidioStreamError::<std::io::Error>::Actions(
        crate::presidio_actions::error::PresidioActionsError::BlockedEntity {
            entity_type: "PERSON".to_owned(),
            guardrail_name: "g".to_owned(),
        },
    );
    assert!(blocked.is_blocked());
    assert_eq!(blocked.status(), 400);
    let closed = PresidioStreamError::<std::io::Error>::ConsumerClosed;
    assert!(!closed.is_blocked());
    assert_eq!(closed.status(), 500);
}
