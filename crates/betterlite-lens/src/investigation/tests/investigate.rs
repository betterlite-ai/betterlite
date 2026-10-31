#![cfg(test)]

use super::fixtures;
use crate::analysis::{AnalysisError, ModelCall};
use crate::investigation::investigate;
use std::sync::Arc;

#[tokio::test]
async fn entrypoint_converts_decode_errors_to_empty_investigation() {
    let claim = fixtures::claim();
    let candidate = fixtures::candidate(vec![String::from("e1")]);
    let examined = vec![fixtures::examined("e1", "nothing to see here")];
    let (model, _) = fixtures::scripted(vec![String::from("not json")]);
    let read = fixtures::never_read();

    let result = investigate(claim, candidate, examined, read, model)
        .await
        .unwrap();
    assert!(result.finding.is_none());
    assert!(result.parts.is_empty());
}

#[tokio::test]
async fn entrypoint_propagates_errors_that_are_not_validation_failures() {
    let claim = fixtures::claim();
    let candidate = fixtures::candidate(vec![String::from("e1")]);
    let examined = vec![fixtures::examined("e1", "nothing to see here")];
    let model: ModelCall = Arc::new(|_| {
        Box::pin(async { Err(AnalysisError::Callback(String::from("model unavailable"))) })
    });
    let read = fixtures::never_read();

    let error = investigate(claim, candidate, examined, read, model)
        .await
        .unwrap_err();
    assert!(matches!(error, AnalysisError::Callback(_)));
}
