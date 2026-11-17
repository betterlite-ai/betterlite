#![cfg(test)]

use std::io::Error;

use betterlite_config::sections::guardrail_identity::enums::PiiAction;
use betterlite_core::api::api_boundaries::ApiLimits;

use super::shared::{
    DELTA_FRAME, Events, Harness, Kind, START_FRAME, bytes, capture_logs, chat, logged_text, ok,
    request_data, spawn_person_analyzer, upstream_failure,
};
use crate::presidio_actions::error::PresidioActionsError;
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

#[tokio::test]
async fn over_budget_bytes_in_the_mask_buffer_are_raised() {
    let harness = Harness::new();
    let mut request = request_data("{}");
    let huge = "x".repeat(ApiLimits::default().max_json_bytes);
    let (sent, result) = harness
        .run(Kind::Mask, &mut request, ok(vec![chat("a"), chat(&huge)]))
        .await;
    assert!(matches!(
        result.unwrap_err(),
        PresidioStreamError::OverBudget { .. }
    ));
    assert!(sent.is_empty());
}

#[tokio::test]
async fn over_budget_items_in_the_mask_buffer_are_raised() {
    let harness = Harness::new();
    let mut request = request_data("{}");
    let events = ok(vec![chat("a"); ApiLimits::default().max_items + 1]);
    let (sent, result) = harness.run(Kind::Mask, &mut request, events).await;
    assert!(matches!(
        result.unwrap_err(),
        PresidioStreamError::OverBudget { .. }
    ));
    assert!(sent.is_empty());
}

#[tokio::test]
async fn a_chunk_count_at_the_ceiling_is_masked() {
    let harness = Harness::new();
    let mut request = request_data("{}");
    let events = ok(vec![chat("a"); ApiLimits::default().max_items]);
    let (sent, result) = harness.run(Kind::Mask, &mut request, events).await;
    result.unwrap();
    assert_eq!(sent.len(), 1);
}

#[tokio::test]
async fn over_budget_bytes_in_the_anthropic_buffer_are_raised() {
    let harness = Harness::new();
    let mut request = request_data("{}");
    let huge = format!(
        "data: {}\n\n",
        "x".repeat(ApiLimits::default().max_json_bytes)
    );
    let events = ok(vec![bytes(START_FRAME), bytes(&huge)]);
    let (sent, result) = harness.run(Kind::Mask, &mut request, events).await;
    assert!(matches!(
        result.unwrap_err(),
        PresidioStreamError::OverBudget { .. }
    ));
    assert!(sent.is_empty());
}

#[tokio::test]
async fn over_budget_items_in_the_anthropic_buffer_are_raised() {
    let harness = Harness::new();
    let mut request = request_data("{}");
    let mut events = vec![bytes(START_FRAME)];
    events.extend(vec![bytes(DELTA_FRAME); ApiLimits::default().max_items]);
    let (sent, result) = harness.run(Kind::Mask, &mut request, ok(events)).await;
    assert!(matches!(
        result.unwrap_err(),
        PresidioStreamError::OverBudget { .. }
    ));
    assert!(sent.is_empty());
}

#[tokio::test]
async fn an_anthropic_stream_at_the_item_ceiling_is_masked() {
    let harness = Harness::new();
    let mut request = request_data("{}");
    let mut events = vec![bytes(START_FRAME)];
    events.extend(vec![bytes(DELTA_FRAME); ApiLimits::default().max_items - 1]);
    let (sent, result) = harness.run(Kind::Mask, &mut request, ok(events)).await;
    result.unwrap();
    assert_eq!(sent, ["bytes rebuilt:MASKED"]);
}

#[test]
fn only_a_blocked_entity_is_a_blocked_error() {
    let blocked = PresidioStreamError::<Error>::Actions(PresidioActionsError::BlockedEntity {
        entity_type: "PERSON".to_owned(),
        guardrail_name: "g".to_owned(),
    });
    assert!(blocked.is_blocked());
    let closed = PresidioStreamError::<Error>::ConsumerClosed;
    assert!(!closed.is_blocked());
}
