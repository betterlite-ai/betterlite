#![cfg(test)]
use crate::analysis::{AnalysisError, ModelCall};
use crate::extraction::{
    extract,
    tests::{claim, execution, model, page, reader},
};
use std::sync::Arc;

#[tokio::test]
async fn entrypoint_invalid_response_is_unassessable() {
    let result = extract(
        claim(),
        execution(),
        reader(vec![page()]),
        model(vec!["not json", "not json"]),
    )
    .await
    .unwrap();
    assert!(result.partial && result.cannot_assess);
    assert!(result.parts.is_empty() && result.observations.is_empty());
}

#[tokio::test]
async fn entrypoint_empty_spans_do_not_invent_issues() {
    let mut empty = page();
    empty.parts.clear();
    let result = extract(claim(), execution(), reader(vec![empty]), model(vec![]))
        .await
        .unwrap();
    assert!(result.cannot_assess);
    assert!(!result.partial);
    assert!(result.observations.is_empty());
}

#[tokio::test]
async fn entrypoint_propagates_callback_failure() {
    let callback: ModelCall =
        Arc::new(|_| Box::pin(async { Err(AnalysisError::Callback(String::from("offline"))) }));
    let result = extract(claim(), execution(), reader(vec![page()]), callback).await;
    assert!(matches!(result, Err(AnalysisError::Callback(_))));
}
