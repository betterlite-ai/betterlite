#![cfg(test)]
use super::cluster::{cluster_batches, settle_page};
use super::fixtures::{candidate, capture_progress, echo_candidates, echo_model, observation};
use crate::analysis::{Candidate, ModelCall, ObservationKind};
use crate::models::execution::Coverage;
use crate::models::messages::ModelResult;
use std::sync::{Arc, Mutex};

fn echo_recording_model() -> (ModelCall, Arc<Mutex<Vec<String>>>) {
    let prompts = Arc::new(Mutex::new(Vec::new()));
    let handle = Arc::clone(&prompts);
    let model: ModelCall = Arc::new(move |request| {
        handle.lock().unwrap().push(request.prompt.clone());
        Box::pin(async move {
            Ok(ModelResult {
                content: echo_candidates(&request.prompt),
                cost: rust_decimal::Decimal::ZERO,
            })
        })
    });
    (model, prompts)
}

#[tokio::test]
async fn accumulates_batches_then_builds_registry_from_all_candidates() {
    let (model, _calls) = echo_model();
    let (progress, _log) = capture_progress();
    let batches = vec![
        vec![observation(
            "a",
            ObservationKind::Issue,
            "first",
            vec!["r0"],
        )],
        vec![observation(
            "b",
            ObservationKind::Issue,
            "second",
            vec!["r1"],
        )],
    ];
    let clusters = cluster_batches(batches, model, progress, Coverage::default())
        .await
        .unwrap();
    let mut check_ids: Vec<_> = clusters
        .candidates
        .iter()
        .map(|candidate| candidate.check_id.clone())
        .collect();
    check_ids.sort();
    assert_eq!(check_ids, vec![String::from("a"), String::from("b")]);
}

#[tokio::test]
async fn keeps_different_kinds_separate_even_when_check_id_matches() {
    let (model, _calls) = echo_model();
    let (progress, _log) = capture_progress();
    let batches = vec![
        vec![observation(
            "shared",
            ObservationKind::Issue,
            "an issue",
            vec!["r0"],
        )],
        vec![observation(
            "shared",
            ObservationKind::Pattern,
            "a pattern",
            vec!["r1"],
        )],
    ];
    let clusters = cluster_batches(batches, model, progress, Coverage::default())
        .await
        .unwrap();
    assert_eq!(clusters.candidates.len(), 2);
    let mut kinds: Vec<_> = clusters
        .candidates
        .iter()
        .map(|candidate| candidate.kind)
        .collect();
    kinds.sort_by_key(|kind| matches!(kind, ObservationKind::Pattern));
    assert_eq!(
        kinds,
        vec![ObservationKind::Issue, ObservationKind::Pattern]
    );
}

#[tokio::test]
async fn sorts_the_registry_even_when_batches_accumulate_in_an_unsorted_order() {
    let (model, _calls) = echo_model();
    let (progress, _log) = capture_progress();
    let batches = vec![
        vec![observation(
            "b",
            ObservationKind::Issue,
            "second",
            vec!["r1"],
        )],
        vec![observation(
            "a",
            ObservationKind::Issue,
            "first",
            vec!["r0"],
        )],
    ];
    let clusters = cluster_batches(batches, model, progress, Coverage::default())
        .await
        .unwrap();
    let check_ids: Vec<_> = clusters
        .candidates
        .iter()
        .map(|candidate| candidate.check_id.clone())
        .collect();
    assert_eq!(check_ids, vec![String::from("a"), String::from("b")]);
}

#[tokio::test]
async fn settle_page_keeps_a_same_check_id_other_kind_entry_out_of_the_prompt() {
    let (model, prompts) = echo_recording_model();
    let registry = vec![candidate(
        "shared",
        ObservationKind::Pattern,
        "unrelated-kind-title",
        "unrelated-kind-hypothesis",
        vec!["r0"],
    )];
    let incoming = vec![candidate(
        "shared",
        ObservationKind::Issue,
        "incoming-title",
        "incoming-hypothesis",
        vec!["r1"],
    )];
    let result: Vec<Candidate> = settle_page(registry, incoming, &model).await.unwrap();
    let sent = prompts.lock().unwrap().clone();
    assert!(
        sent.iter()
            .all(|prompt| !prompt.contains("unrelated-kind-title"))
    );
    assert_eq!(result[0].check_id, "shared");
    assert_eq!(result[0].kind, ObservationKind::Pattern);
    assert_eq!(result[0].title, "unrelated-kind-title");
}

#[tokio::test]
async fn reports_grouping_observations_progress_per_accumulated_batch() {
    let (model, _calls) = echo_model();
    let (progress, log) = capture_progress();
    let batches = vec![
        vec![observation(
            "a",
            ObservationKind::Issue,
            "first",
            vec!["r0"],
        )],
        vec![observation(
            "b",
            ObservationKind::Issue,
            "second",
            vec!["r1"],
        )],
    ];
    cluster_batches(batches, model, progress, Coverage::default())
        .await
        .unwrap();
    let stages: Vec<_> = log
        .lock()
        .unwrap()
        .iter()
        .map(|(stage, coverage)| (stage.clone(), coverage.grouped_batches))
        .collect();
    assert_eq!(
        stages,
        vec![
            (String::from("Grouping observations"), 0),
            (String::from("Grouping observations"), 1),
        ]
    );
}
