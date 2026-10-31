#![cfg(test)]

use super::fixtures;
use crate::analysis::{Examined, Observation, ObservationKind};
use crate::investigation::loop_state::{self, Context};
use serde::Deserialize;

#[derive(Deserialize)]
struct ReadCount {
    completed_read_count: i64,
}

#[tokio::test]
async fn full_loop_pages_through_navigation_before_submitting() {
    let claim = fixtures::claim();
    let candidate = fixtures::candidate(vec![String::from("e1")]);
    let examined = vec![fixtures::examined("e1", "the original quote")];
    let read = fixtures::never_read();
    let (model, prompts) = fixtures::scripted(vec![
        fixtures::observations_decision(0),
        fixtures::catalog_decision(0),
        fixtures::feedback_decision(0),
        fixtures::submit_decision("e1", "s1", "the original quote"),
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

    assert_eq!(prompts.lock().unwrap().len(), 4);
    let read_counts: Vec<i64> = prompts
        .lock()
        .unwrap()
        .iter()
        .map(|prompt| {
            let decoded: ReadCount = serde_json::from_str(prompt).unwrap();
            decoded.completed_read_count
        })
        .collect();
    assert_eq!(read_counts, vec![0, 1, 2, 3]);
}

#[tokio::test]
async fn catalog_partitioning_sizes_only_the_execution_not_the_trace_content() {
    let claim = fixtures::claim();
    let candidate = fixtures::candidate(vec![String::from("e1")]);
    let large_content = format!("the original quote{}", "x".repeat(20_000));
    let examined = vec![fixtures::examined("e1", &large_content)];
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
async fn an_execution_can_contribute_more_than_one_matching_observation() {
    let claim = fixtures::claim();
    let candidate = fixtures::candidate(vec![String::from("e1")]);
    let observation = Observation {
        check_id: String::from("check"),
        kind: ObservationKind::Issue,
        summary: String::from("a supporting observation"),
        evidence: Vec::new(),
    };
    let examined = vec![Examined {
        execution: fixtures::execution("e1"),
        observations: vec![observation.clone(), observation],
        parts: vec![fixtures::part("e1", "s1", "the original quote")],
        partial: false,
        cannot_assess: false,
    }];
    let read = fixtures::never_read();
    let (model, prompts) = fixtures::scripted(vec![fixtures::inconclusive_decision()]);

    let context = Context {
        claim: &claim,
        candidate: &candidate,
        examined: &examined,
        read: &read,
        model: &model,
    };
    let result = loop_state::run(context).await.unwrap();
    assert!(result.finding.is_none());
    assert_eq!(prompts.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn an_empty_candidate_title_still_builds_a_prompt() {
    let claim = fixtures::claim();
    let mut candidate = fixtures::candidate(vec![String::from("e1")]);
    candidate.title = String::new();
    let examined = vec![fixtures::examined("e1", "the original quote")];
    let read = fixtures::never_read();
    let (model, prompts) = fixtures::scripted(vec![fixtures::inconclusive_decision()]);

    let context = Context {
        claim: &claim,
        candidate: &candidate,
        examined: &examined,
        read: &read,
        model: &model,
    };
    let result = loop_state::run(context).await.unwrap();
    assert!(result.finding.is_none());
    assert_eq!(prompts.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn mismatched_execution_ids_still_run_the_loop() {
    let claim = fixtures::claim();
    let candidate = fixtures::candidate(vec![String::from("missing")]);
    let examined = vec![fixtures::examined("e1", "the original quote")];
    let read = fixtures::never_read();
    let (model, prompts) = fixtures::scripted(vec![fixtures::inconclusive_decision()]);

    let context = Context {
        claim: &claim,
        candidate: &candidate,
        examined: &examined,
        read: &read,
        model: &model,
    };
    let result = loop_state::run(context).await.unwrap();
    assert!(result.finding.is_none());
    assert_eq!(prompts.lock().unwrap().len(), 1);
}
