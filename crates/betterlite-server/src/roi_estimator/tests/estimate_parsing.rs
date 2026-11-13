#![cfg(test)]

use betterlite_roi::records::{ROIEstimate, ROIEstimateStatus};
use rstest::rstest;
use serde_json::{Value, json};

use crate::roi_estimator::Estimator;
use crate::roi_estimator::error::EstimatorError;

use super::support::{ModelSupport, caller, ok_response, pull, settings};

async fn run_estimate(response: Value) -> Result<ROIEstimate, EstimatorError> {
    let (complete, _requests) = caller(Ok(response));
    let estimator = Estimator::new(settings("gpt-5.2", "Estimate this."), complete, Some(&[]));
    estimator.estimate(&pull(), &mut ModelSupport::none()).await
}

#[rstest]
#[case::plain(r#"{"hours": 3.5, "reasoning": "Small change."}"#)]
#[case::fenced("Sure:\n```json\n{\"hours\": 3.5, \"reasoning\": \"Small change.\"}\n```")]
#[case::prose_before("The estimate follows. {\"hours\": 3.5, \"reasoning\": \"Small change.\"}")]
#[tokio::test]
async fn estimate_parses_the_json_object_out_of_the_reply(#[case] content: &str) {
    let estimate = run_estimate(ok_response(content)).await.unwrap();
    assert_eq!(estimate.status, ROIEstimateStatus::Estimated);
    assert_eq!(estimate.hours, Some(3.5));
    assert_eq!(estimate.reasoning, "Small change.");
}

#[tokio::test]
async fn estimate_uses_the_first_complete_object_in_the_reply() {
    let estimate = run_estimate(ok_response(
        "{\"hours\": 1, \"reasoning\": \"first\"} and {\"hours\": 2, \"reasoning\": \"second\"}",
    ))
    .await
    .unwrap();
    assert_eq!(estimate.hours, Some(1.0));
    assert_eq!(estimate.reasoning, "first");
}

#[rstest]
#[case::not_json(ok_response("not json"))]
#[case::extra_field(ok_response(r#"{"hours": 1, "reasoning": "x", "extra": 2}"#))]
#[case::negative_hours(ok_response(r#"{"hours": -1, "reasoning": "x"}"#))]
#[case::empty_reasoning(ok_response(r#"{"hours": 1, "reasoning": "  "}"#))]
#[case::length_finish(ok_response_with_finish("length"))]
#[case::null_content(json!({"choices": [{"finish_reason": "stop", "message": {"content": null}}]}))]
#[case::no_choices(json!({"choices": []}))]
#[case::no_content_key(json!({"choices": [{"finish_reason": "stop", "message": {}}]}))]
#[case::wrong_schema(json!({"choices": "nope"}))]
#[tokio::test]
async fn estimate_rejects_replies_that_do_not_carry_a_valid_estimate(#[case] response: Value) {
    assert!(run_estimate(response).await.is_err());
}

fn ok_response_with_finish(finish_reason: &str) -> Value {
    json!({
        "choices": [{
            "finish_reason": finish_reason,
            "message": {"content": "{\"hours\": 1, \"reasoning\": \"x\"}"},
        }],
    })
}
