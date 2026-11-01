#![cfg(test)]
use super::fixtures::{candidate, model};
use super::merge::merge_candidates;
use crate::analysis::ObservationKind;

#[tokio::test]
async fn empty_execution_ids_is_discarded_and_original_falls_back() {
    let response = r#"{"candidates":[{"check_id":"a","kind":"issue","title":"x","hypothesis":"x","execution_ids":[]}]}"#;
    let model_call = model(vec![String::from(response)]);
    let original = candidate("a", ObservationKind::Issue, "a", "a", vec!["r0"]);
    let (active, settled) = merge_candidates(vec![original], 0, &model_call)
        .await
        .unwrap();
    assert!(settled.is_empty());
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].check_id, "a");
    assert_eq!(active[0].execution_ids, vec![String::from("r0")]);
}

#[tokio::test]
async fn identity_absent_from_the_map_is_discarded_and_original_falls_back() {
    let response = r#"{"candidates":[{"check_id":"a","kind":"issue","title":"x","hypothesis":"x","execution_ids":["p99"]}]}"#;
    let model_call = model(vec![String::from(response)]);
    let original = candidate("a", ObservationKind::Issue, "a", "a", vec!["r0"]);
    let (active, settled) = merge_candidates(vec![original], 0, &model_call)
        .await
        .unwrap();
    assert!(settled.is_empty());
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].check_id, "a");
}

#[tokio::test]
async fn mismatched_check_id_against_the_identity_is_discarded_and_original_falls_back() {
    let response = r#"{"candidates":[{"check_id":"b","kind":"issue","title":"x","hypothesis":"x","execution_ids":["p0"]}]}"#;
    let model_call = model(vec![String::from(response)]);
    let original = candidate("a", ObservationKind::Issue, "a", "a", vec!["r0"]);
    let (active, settled) = merge_candidates(vec![original], 0, &model_call)
        .await
        .unwrap();
    assert!(settled.is_empty());
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].check_id, "a");
}

#[tokio::test]
async fn mismatched_kind_against_the_identity_is_discarded_and_original_falls_back() {
    let response = r#"{"candidates":[{"check_id":"a","kind":"pattern","title":"x","hypothesis":"x","execution_ids":["p0"]}]}"#;
    let model_call = model(vec![String::from(response)]);
    let original = candidate("a", ObservationKind::Issue, "a", "a", vec!["r0"]);
    let (active, settled) = merge_candidates(vec![original], 0, &model_call)
        .await
        .unwrap();
    assert!(settled.is_empty());
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].kind, ObservationKind::Issue);
}
