#![cfg(test)]
use super::fixtures::{candidate, echo_model};
use super::merge::merge_candidates;
use crate::analysis::ObservationKind;

#[tokio::test]
async fn splits_prior_candidates_as_settled_and_new_candidates_as_active() {
    let (model, _calls) = echo_model();
    let prior = candidate("a", ObservationKind::Issue, "prior", "prior", vec!["r0"]);
    let incoming = candidate("b", ObservationKind::Issue, "new", "new", vec!["r1"]);
    let (active, settled) = merge_candidates(vec![prior, incoming], 1, &model)
        .await
        .unwrap();
    assert_eq!(settled.len(), 1);
    assert_eq!(settled[0].check_id, "a");
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].check_id, "b");
}

#[tokio::test]
async fn zero_prior_count_makes_every_candidate_active() {
    let (model, _calls) = echo_model();
    let first = candidate("a", ObservationKind::Issue, "a", "a", vec!["r0"]);
    let second = candidate("b", ObservationKind::Issue, "b", "b", vec!["r1"]);
    let (active, settled) = merge_candidates(vec![first, second], 0, &model)
        .await
        .unwrap();
    assert!(settled.is_empty());
    assert_eq!(active.len(), 2);
}
