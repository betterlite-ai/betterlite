#![cfg(test)]
use super::fixtures::{candidate, model};
use super::merge::merge_candidates;
use crate::analysis::ObservationKind;

#[tokio::test]
async fn duplicate_reference_across_groups_triggers_repair_and_retries() {
    let duplicated = r#"{"candidates":[{"check_id":"a","kind":"issue","title":"a","hypothesis":"a","execution_ids":["p0"]},{"check_id":"a","kind":"issue","title":"a-again","hypothesis":"a-again","execution_ids":["p0"]}]}"#;
    let repaired = r#"{"candidates":[{"check_id":"a","kind":"issue","title":"a","hypothesis":"a","execution_ids":["p0"]}]}"#;
    let responses = vec![String::from(duplicated), String::from(repaired)];
    let model_call = model(responses);
    let candidates = vec![candidate("a", ObservationKind::Issue, "a", "a", vec!["r0"])];
    let (active, settled) = merge_candidates(candidates, 0, &model_call).await.unwrap();
    assert!(settled.is_empty());
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].execution_ids, vec![String::from("r0")]);
}

#[tokio::test]
async fn identity_absent_from_every_group_is_not_an_error() {
    let response = r#"{"candidates":[{"check_id":"a","kind":"issue","title":"a","hypothesis":"a","execution_ids":["p0"]}]}"#;
    let model_call = model(vec![String::from(response)]);
    let candidates = vec![
        candidate("a", ObservationKind::Issue, "a", "a", vec!["r0"]),
        candidate("b", ObservationKind::Issue, "b", "b", vec!["r1"]),
    ];
    let (active, settled) = merge_candidates(candidates, 0, &model_call).await.unwrap();
    let mut check_ids: Vec<_> = active
        .iter()
        .chain(settled.iter())
        .map(|candidate| candidate.check_id.clone())
        .collect();
    check_ids.sort();
    assert_eq!(check_ids, vec![String::from("a"), String::from("b")]);
}
