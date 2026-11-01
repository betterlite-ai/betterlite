#![cfg(test)]
use super::batches::observation_batches;
use super::fixtures::observation;
use crate::analysis::ObservationKind;
use std::iter::repeat_with;

#[test]
fn sorts_by_check_id_then_kind_before_partitioning() {
    let items = vec![
        observation(
            "b",
            ObservationKind::Pattern,
            "second check, pattern",
            vec!["r0"],
        ),
        observation(
            "a",
            ObservationKind::Pattern,
            "first check, pattern",
            vec!["r0"],
        ),
        observation(
            "a",
            ObservationKind::Issue,
            "first check, issue",
            vec!["r0"],
        ),
    ];
    let batches = observation_batches(&items).unwrap();
    let flattened: Vec<(String, ObservationKind)> = batches
        .into_iter()
        .flatten()
        .map(|item| (item.check_id, item.kind))
        .collect();
    assert_eq!(
        flattened,
        vec![
            (String::from("a"), ObservationKind::Issue),
            (String::from("a"), ObservationKind::Pattern),
            (String::from("b"), ObservationKind::Pattern),
        ]
    );
}

#[test]
fn partitions_by_serialized_json_size_at_16000_bytes() {
    let single = observation("check", ObservationKind::Issue, &"x".repeat(100), vec![]);
    let size = serde_json::to_vec(&single).unwrap().len();
    let mut per_batch = 0_usize;
    while size.saturating_mul(per_batch.saturating_add(1)) <= 16000 {
        per_batch = per_batch.saturating_add(1);
    }
    let items: Vec<_> =
        repeat_with(|| observation("check", ObservationKind::Issue, &"x".repeat(100), vec![]))
            .take(per_batch.saturating_add(1))
            .collect();
    let batches = observation_batches(&items).unwrap();
    assert_eq!(batches.len(), 2);
    assert!(batches[0].len() <= per_batch);
    assert_eq!(batches.iter().map(Vec::len).sum::<usize>(), items.len());
}

#[test]
fn empty_observations_returns_no_batches() {
    let batches = observation_batches(&[]).unwrap();
    assert!(batches.is_empty());
}
