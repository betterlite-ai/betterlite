#![cfg(test)]

use std::error::Error as _;

use betterlite_roi::records::ROIEstimate;

use crate::roi_estimator::error::EstimatorError;
use crate::roi_estimator::{CompletionObject, Estimator};
use crate::roi_github::error::SourceError;

use super::support::{ModelSupport, caller, completion, ok_response, pull, settings};

const MESSAGE: &str =
    "The estimator did not return valid hours and reasoning. Check the selected model and prompt.";

async fn run_estimate(
    response: Result<CompletionObject, SourceError>,
) -> Result<ROIEstimate, EstimatorError> {
    let (complete, _requests) = caller(response);
    let estimator = Estimator::new(settings("gpt-5.2", "Estimate this."), complete, Some(&[]));
    estimator.estimate(&pull(), &mut ModelSupport::none()).await
}

#[tokio::test]
async fn estimate_reports_the_litellm_message_when_the_completion_call_fails() {
    let error = run_estimate(Err(SourceError::Http {
        label: "estimator",
        status: 502,
    }))
    .await
    .unwrap_err();
    assert_eq!(error.to_string(), MESSAGE);
    assert!(error.source().is_some());
}

#[tokio::test]
async fn estimate_reports_the_litellm_message_when_the_reply_is_not_json() {
    let error = run_estimate(Ok(ok_response("not json"))).await.unwrap_err();
    assert_eq!(error.to_string(), MESSAGE);
}

#[tokio::test]
async fn estimate_reports_the_litellm_message_when_the_reply_is_incomplete() {
    let incomplete = completion(
        r#"{"choices": [{"finish_reason": "length", "message": {"content": "{\"hours\": 1, \"reasoning\": \"x\"}"}}]}"#,
    );
    let error = run_estimate(Ok(incomplete)).await.unwrap_err();
    assert_eq!(error.to_string(), MESSAGE);
}

#[tokio::test]
async fn estimate_reports_the_litellm_message_when_the_envelope_is_invalid() {
    let error = run_estimate(Ok(completion(r#"{"choices": 3}"#)))
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), MESSAGE);
}

#[tokio::test]
async fn estimate_reports_the_litellm_message_when_the_estimate_is_invalid() {
    let invalid = ok_response(r#"{"hours": -1, "reasoning": "x"}"#);
    let error = run_estimate(Ok(invalid)).await.unwrap_err();
    assert_eq!(error.to_string(), MESSAGE);
}
