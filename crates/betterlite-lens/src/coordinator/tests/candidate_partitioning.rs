#![cfg(test)]
use super::cluster::{candidate_size, consolidate, settle};
use super::fixtures::{candidate, echo_model, observation};
use crate::analysis::{Candidate, ObservationKind};

fn sized_candidate(check_id: &str) -> Candidate {
    candidate(
        check_id,
        ObservationKind::Issue,
        "",
        &"h".repeat(1800),
        vec!["r0"],
    )
}

fn per_page(limit: usize) -> usize {
    let size = candidate_size(&sized_candidate("check0"));
    let mut count = 0_usize;
    while size.saturating_mul(count.saturating_add(1)) <= limit {
        count = count.saturating_add(1);
    }
    count
}

#[tokio::test]
async fn accumulation_fits_one_page_within_16000_bytes() {
    let (model, calls) = echo_model();
    let count = per_page(16000);
    let previous: Vec<_> = (0..count)
        .map(|index| sized_candidate(&format!("check{index}")))
        .collect();
    let batch = vec![observation(
        "new",
        ObservationKind::Issue,
        "new candidate",
        vec!["r9"],
    )];
    consolidate(batch, previous, &model).await.unwrap();
    assert_eq!(*calls.lock().unwrap(), 1);
}

#[tokio::test]
async fn accumulation_splits_into_two_pages_past_16000_bytes() {
    let (model, calls) = echo_model();
    let count = per_page(16000) + 1;
    let previous: Vec<_> = (0..count)
        .map(|index| sized_candidate(&format!("check{index}")))
        .collect();
    let batch = vec![observation(
        "new",
        ObservationKind::Issue,
        "new candidate",
        vec!["r9"],
    )];
    let result = consolidate(batch, previous, &model).await.unwrap();
    assert_eq!(*calls.lock().unwrap(), 2);
    assert_eq!(result.len(), count + 1);
}

#[tokio::test]
async fn registry_comparison_fits_one_page_within_8000_bytes() {
    let (model, calls) = echo_model();
    let count = per_page(8000);
    let candidates: Vec<_> = (0..count)
        .map(|index| sized_candidate(&format!("check{index}")))
        .collect();
    let registry = settle(candidates, &model).await.unwrap();
    assert_eq!(*calls.lock().unwrap(), 1);
    assert_eq!(registry.len(), count);
}

#[tokio::test]
async fn registry_comparison_splits_into_two_pages_past_8000_bytes() {
    let (model, calls) = echo_model();
    let count = per_page(8000) + 1;
    let candidates: Vec<_> = (0..count)
        .map(|index| sized_candidate(&format!("check{index}")))
        .collect();
    let registry = settle(candidates, &model).await.unwrap();
    assert_eq!(*calls.lock().unwrap(), 2);
    let mut check_ids: Vec<_> = registry.iter().map(|item| item.check_id.clone()).collect();
    check_ids.sort();
    let mut expected: Vec<String> = (0..count).map(|index| format!("check{index}")).collect();
    expected.sort();
    assert_eq!(check_ids, expected);
}
