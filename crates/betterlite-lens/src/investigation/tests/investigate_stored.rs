#![cfg(test)]

use super::fixtures;
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
