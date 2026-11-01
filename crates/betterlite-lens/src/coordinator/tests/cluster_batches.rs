#![cfg(test)]
use super::cluster::cluster_batches;
use super::fixtures::{capture_progress, echo_model, observation};
use crate::analysis::ObservationKind;
use crate::models::execution::Coverage;

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
