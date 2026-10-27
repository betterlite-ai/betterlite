#![cfg(test)]
use crate::extraction::{
    extract,
    tests::{OBSERVATION, claim, execution, model, page, reader},
};
use crate::models::messages::Claim;
use rstest::rstest;

#[rstest]
#[case::invalid(vec![r#"{"feedback_page":9}"#, OBSERVATION])]
#[case::cycle(vec![r#"{"feedback_page":1}"#, r#"{"feedback_page":0}"#, OBSERVATION])]
#[tokio::test]
async fn forces_final_decision(#[case] replies: Vec<&str>) {
    let mut input = claim();
    let original: Claim =
        serde_json::from_str(include_str!("../../analysis/fixtures/claim.json")).unwrap();
    let mut finding = original.findings[0].clone();
    finding.reason = "x".repeat(4100);
    input.findings = vec![finding.clone(), finding];
    let result = extract(input, execution(), reader(vec![page()]), model(replies))
        .await
        .unwrap();
    assert_eq!(result.observations.len(), 1);
    assert!(!result.cannot_assess);
}
