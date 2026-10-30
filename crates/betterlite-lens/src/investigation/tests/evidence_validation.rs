#![cfg(test)]

use super::fixtures;
use crate::investigation::loop_state::{self, Context};
use crate::models::execution::ExecutionContent;

#[tokio::test]
async fn evidence_validation_accepts_an_exact_quote_match() {
    let claim = fixtures::claim();
    let candidate = fixtures::candidate(vec![String::from("e1")]);
    let examined = vec![fixtures::examined("e1", "the original quote")];
    let read = fixtures::never_read();
    let (model, _) = fixtures::scripted(vec![fixtures::submit_decision(
        "e1",
        "s1",
        "the original quote",
    )]);

    let context = Context {
        claim: &claim,
        candidate: &candidate,
        examined: &examined,
        read: &read,
        model: &model,
    };
    let result = loop_state::run(context).await.unwrap();
    assert!(result.finding.is_some());
}

#[tokio::test]
async fn evidence_validation_rejects_a_quote_that_matches_no_span() {
    let claim = fixtures::claim();
    let candidate = fixtures::candidate(vec![String::from("e1")]);
    let examined = vec![fixtures::examined("e1", "the original quote")];
    let read = fixtures::never_read();
    let (model, _) = fixtures::scripted(vec![fixtures::submit_decision(
        "e1",
        "s1",
        "a quote nobody ever recorded",
    )]);

    let context = Context {
        claim: &claim,
        candidate: &candidate,
        examined: &examined,
        read: &read,
        model: &model,
    };
    let result = loop_state::run(context).await.unwrap();
    assert!(result.finding.is_none());
}

#[tokio::test]
async fn evidence_validation_falls_back_to_the_trace_store_once_a_span_leaves_the_current_page() {
    let claim = fixtures::claim();
    let candidate = fixtures::candidate(vec![String::from("e1")]);
    let examined = vec![fixtures::examined("e1", "unrelated starting content")];
    let first_page = ExecutionContent {
        execution: fixtures::execution("e1"),
        parts: vec![fixtures::part("e1", "s-read-1", "first read content")],
        next_cursor: None,
        partial: false,
    };
    let second_page = ExecutionContent {
        execution: fixtures::execution("e1"),
        parts: vec![fixtures::part("e1", "s-read-2", "second read content")],
        next_cursor: None,
        partial: false,
    };
    let (read, _) = fixtures::sequential_read(vec![first_page, second_page]);
    let (model, _) = fixtures::scripted(vec![
        fixtures::read_decision("e1", "", 0),
        fixtures::read_decision("e1", "page-two", 0),
        fixtures::submit_decision("e1", "s-read-1", "first read content"),
    ]);

    let context = Context {
        claim: &claim,
        candidate: &candidate,
        examined: &examined,
        read: &read,
        model: &model,
    };
    let result = loop_state::run(context).await.unwrap();
    assert!(result.finding.is_some());
}
