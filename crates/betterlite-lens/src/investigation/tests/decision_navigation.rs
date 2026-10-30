#![cfg(test)]

use super::fixtures;
use crate::investigation::loop_state::{self, Context};
use crate::models::execution::ExecutionContent;
use serde::Deserialize;

#[derive(Deserialize)]
struct Pages {
    #[serde(rename = "observation_page")]
    observations: i64,
    #[serde(rename = "catalog_page")]
    catalog: i64,
    #[serde(rename = "feedback_page")]
    feedback: i64,
}

#[tokio::test]
async fn navigation_tracks_observation_catalog_and_feedback_pages_independently() {
    let claim = fixtures::claim();
    let candidate = fixtures::candidate(vec![String::from("e1")]);
    let examined = vec![fixtures::examined("e1", "the original quote")];
    let read = fixtures::never_read();
    let (model, prompts) = fixtures::scripted(vec![
        fixtures::observations_decision(0),
        fixtures::catalog_decision(0),
        fixtures::feedback_decision(0),
        fixtures::inconclusive_decision(),
    ]);

    let context = Context {
        claim: &claim,
        candidate: &candidate,
        examined: &examined,
        read: &read,
        model: &model,
    };
    loop_state::run(context).await.unwrap();

    let pages: Vec<(i64, i64, i64)> = prompts
        .lock()
        .unwrap()
        .iter()
        .map(|prompt| {
            let decoded: Pages = serde_json::from_str(prompt).unwrap();
            (decoded.observations, decoded.catalog, decoded.feedback)
        })
        .collect();
    assert_eq!(pages, vec![(0, 0, 0), (0, 0, 0), (0, 0, 0), (0, 0, 0)]);
}

#[tokio::test]
async fn read_action_terminates_immediately_for_an_unknown_execution_id() {
    let claim = fixtures::claim();
    let candidate = fixtures::candidate(vec![String::from("e1")]);
    let examined = vec![fixtures::examined("e1", "the original quote")];
    let read = fixtures::never_read();
    let (model, prompts) =
        fixtures::scripted(vec![fixtures::read_decision("unknown-execution", "", 0)]);

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
async fn read_action_checks_membership_and_passes_cursor_and_offset_to_read() {
    let claim = fixtures::claim();
    let candidate = fixtures::candidate(vec![String::from("e1")]);
    let examined = vec![fixtures::examined("e1", "the original quote")];
    let page = ExecutionContent {
        execution: fixtures::execution("e1"),
        parts: vec![fixtures::part("e1", "s-read", "freshly read content")],
        next_cursor: None,
        partial: false,
    };
    let (read, calls) = fixtures::sequential_read(vec![page]);
    let (model, _) = fixtures::scripted(vec![
        fixtures::read_decision("e1", "abc", 7),
        fixtures::inconclusive_decision(),
    ]);

    let context = Context {
        claim: &claim,
        candidate: &candidate,
        examined: &examined,
        read: &read,
        model: &model,
    };
    loop_state::run(context).await.unwrap();

    let captured = calls.lock().unwrap();
    assert_eq!(
        captured.as_slice(),
        [fixtures::ReadCall {
            execution_id: String::from("e1"),
            cursor: String::from("abc"),
            offset: 7,
        }]
    );
}
