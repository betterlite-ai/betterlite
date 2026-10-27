#![cfg(test)]
use crate::extraction::{
    extract,
    tests::{claim, execution, model, page, reader},
};

#[tokio::test]
async fn empty_evidence_marks_unassessable() {
    let result = extract(
        claim(),
        execution(),
        reader(vec![page()]),
        model(vec![
            r#"{"observations":[{"check_id":"expected_behavior","summary":"unsupported"}]}"#,
        ]),
    )
    .await
    .unwrap();
    assert!(result.observations.is_empty());
    assert!(result.partial && result.cannot_assess);
}
