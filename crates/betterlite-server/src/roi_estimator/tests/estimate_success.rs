#![cfg(test)]

use betterlite_roi::estimator::models::REASONING_MAX_CHARS;
use betterlite_roi::records::ROIEstimateStatus;
use serde_json::json;

use crate::roi_estimator::Estimator;

use super::support::{ModelSupport, caller, ok_response, pull, settings};

async fn run_estimate(content: &str) -> betterlite_roi::records::ROIEstimate {
    let (complete, _requests) = caller(Ok(ok_response(content)));
    let estimator = Estimator::new(settings("gpt-5.2", "Estimate this."), complete, Some(&[]));
    estimator
        .estimate(&pull(), &mut ModelSupport::none())
        .await
        .unwrap()
}

#[tokio::test]
async fn estimate_returns_the_full_litellm_estimate_record() {
    let estimate = run_estimate(r#"{"hours": 3.5, "reasoning": "Small change."}"#).await;
    assert_eq!(
        serde_json::to_value(&estimate).unwrap(),
        json!({
            "status": "estimated",
            "hours": 3.5,
            "reasoning": "Small change.",
            "model": "gpt-5.2",
            "evidence_source": "pr_metadata",
            "effort_basis": "without_ai",
            "cached": false,
        })
    );
    assert_eq!(estimate.status, ROIEstimateStatus::Estimated);
}

#[tokio::test]
async fn estimate_truncates_reasoning_to_twelve_thousand_characters() {
    let content = json!({"hours": 1, "reasoning": "é".repeat(REASONING_MAX_CHARS + 1)}).to_string();
    let estimate = run_estimate(&content).await;
    assert_eq!(estimate.reasoning, "é".repeat(REASONING_MAX_CHARS));
}

#[tokio::test]
async fn estimate_keeps_reasoning_at_exactly_twelve_thousand_characters() {
    let content = json!({"hours": 1, "reasoning": "é".repeat(REASONING_MAX_CHARS)}).to_string();
    let estimate = run_estimate(&content).await;
    assert_eq!(estimate.reasoning, "é".repeat(REASONING_MAX_CHARS));
}

#[tokio::test]
async fn estimate_renders_integer_hours_as_a_float() {
    let estimate = run_estimate(r#"{"hours": 3, "reasoning": "Small change."}"#).await;
    assert_eq!(estimate.hours, Some(3.0));
    assert!(
        serde_json::to_string(&estimate)
            .unwrap()
            .contains("\"hours\":3.0")
    );
}
