#![cfg(test)]

use betterlite_roi::estimator::extraction::MAX_REPLY_BYTES;
use betterlite_roi::records::{ROIEstimate, ROIEstimateStatus};
use rstest::rstest;

use crate::roi_estimator::error::EstimatorError;
use crate::roi_estimator::{CompletionObject, Estimator};

use super::support::{
    ModelSupport, assert_hours, caller, completion, envelope, ok_response, pull, settings,
};

const VALID: &str = r#"{"hours": 1, "reasoning": "x"}"#;

async fn run_estimate(response: CompletionObject) -> Result<ROIEstimate, EstimatorError> {
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
    assert_hours(&estimate, 3.5_f64);
    assert_eq!(estimate.reasoning, "Small change.");
}

#[tokio::test]
async fn estimate_uses_the_first_complete_object_in_the_reply() {
    let estimate = run_estimate(ok_response(
        "{\"hours\": 1, \"reasoning\": \"first\"} and {\"hours\": 2, \"reasoning\": \"second\"}",
    ))
    .await
    .unwrap();
    assert_hours(&estimate, 1.0_f64);
    assert_eq!(estimate.reasoning, "first");
}

#[tokio::test]
async fn estimate_keeps_the_last_value_of_a_repeated_hours_key() {
    let estimate = run_estimate(ok_response(
        r#"{"hours": 1, "hours": 2, "reasoning": "Small change."}"#,
    ))
    .await
    .unwrap();
    assert_eq!(estimate.status, ROIEstimateStatus::Estimated);
    assert_hours(&estimate, 2.0_f64);
}

#[rstest]
#[case::nan_hours(r#"{"hours": NaN, "hours": 2, "reasoning": "x"}"#)]
#[case::negative_infinity_hours(r#"{"hours": -Infinity, "hours": 2, "reasoning": "x"}"#)]
#[case::infinity_reasoning(r#"{"hours": 2, "reasoning": Infinity, "reasoning": "x"}"#)]
#[tokio::test]
async fn estimate_keeps_the_last_value_past_a_python_constant(#[case] content: &str) {
    let estimate = run_estimate(ok_response(content)).await.unwrap();
    assert_eq!(estimate.status, ROIEstimateStatus::Estimated);
    assert_hours(&estimate, 2.0_f64);
    assert_eq!(estimate.reasoning, "x");
}

#[rstest]
#[case::nan(r#"{"hours": 2, "hours": NaN, "reasoning": "x"}"#)]
#[case::infinity(r#"{"hours": 2, "hours": Infinity, "reasoning": "x"}"#)]
#[case::negative_infinity(r#"{"hours": 2, "hours": -Infinity, "reasoning": "x"}"#)]
#[tokio::test]
async fn estimate_rejects_a_reply_whose_last_hours_is_a_python_constant(#[case] content: &str) {
    let _error = run_estimate(ok_response(content)).await.unwrap_err();
}

#[rstest]
#[case::null(envelope(None, Some(VALID)))]
#[case::missing(completion(
    r#"{"choices": [{"message": {"content": "{\"hours\": 1, \"reasoning\": \"x\"}"}}]}"#
))]
#[tokio::test]
async fn estimate_accepts_a_choice_whose_finish_reason_is_null_or_missing(
    #[case] response: CompletionObject,
) {
    let estimate = run_estimate(response).await.unwrap();
    assert_eq!(estimate.status, ROIEstimateStatus::Estimated);
    assert_hours(&estimate, 1.0_f64);
}

#[tokio::test]
async fn estimate_ignores_the_extra_keys_of_a_gateway_response() {
    let estimate = run_estimate(completion(
        r#"{"id": "chatcmpl-9", "object": "chat.completion", "created": 1760000000, "model": "gpt-5.2", "choices": [{"index": 0, "finish_reason": "stop", "message": {"role": "assistant", "content": "{\"hours\": 1, \"reasoning\": \"x\"}", "tool_calls": null}}], "usage": {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15}}"#,
    ))
    .await
    .unwrap();
    assert_eq!(estimate.status, ROIEstimateStatus::Estimated);
    assert_hours(&estimate, 1.0_f64);
}

#[rstest]
#[case::not_json(ok_response("not json"))]
#[case::extra_field(ok_response(r#"{"hours": 1, "reasoning": "x", "extra": 2}"#))]
#[case::negative_hours(ok_response(r#"{"hours": -1, "reasoning": "x"}"#))]
#[case::empty_reasoning(ok_response(r#"{"hours": 1, "reasoning": "  "}"#))]
#[case::length_finish(ok_response_with_finish("length"))]
#[case::null_content(envelope(Some("stop"), None))]
#[case::no_choices(completion(r#"{"choices": []}"#))]
#[case::no_content_key(completion(r#"{"choices": [{"finish_reason": "stop", "message": {}}]}"#))]
#[case::wrong_schema(completion(r#"{"choices": "nope"}"#))]
#[case::nan_object_before_a_valid_one(ok_response(
    r#"{"hours": NaN, "reasoning": "x"} {"hours": 1, "reasoning": "y"}"#
))]
#[case::reply_over_the_byte_limit(over_limit_response())]
#[tokio::test]
async fn estimate_rejects_replies_that_do_not_carry_a_valid_estimate(
    #[case] response: CompletionObject,
) {
    let _error = run_estimate(response).await.unwrap_err();
}

fn ok_response_with_finish(finish_reason: &str) -> CompletionObject {
    envelope(Some(finish_reason), Some(VALID))
}

fn over_limit_response() -> CompletionObject {
    ok_response(&format!(
        r#"{{"hours": 1, "reasoning": "x"}}{}"#,
        " ".repeat(MAX_REPLY_BYTES)
    ))
}
