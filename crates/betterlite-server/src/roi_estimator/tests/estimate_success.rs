#![cfg(test)]

use betterlite_roi::estimator::models::REASONING_MAX_CHARS;
use betterlite_roi::records::{ROIEstimate, ROIEstimateStatus};

use crate::roi_estimator::Estimator;

use super::support::{ModelSupport, assert_hours, caller, ok_response, pull, settings};

async fn run_estimate(content: &str) -> ROIEstimate {
    let (complete, _requests) = caller(Ok(ok_response(content)));
    let estimator = Estimator::new(settings("gpt-5.2", "Estimate this."), complete, Some(&[]));
    estimator
        .estimate(&pull(), &mut ModelSupport::none())
        .await
        .unwrap()
}

fn content_with_reasoning(reasoning: &str) -> String {
    let quoted = serde_json::to_string(reasoning).unwrap();
    format!(r#"{{"hours": 1, "reasoning": {quoted}}}"#)
}

#[tokio::test]
async fn estimate_returns_the_full_litellm_estimate_record() {
    let estimate = run_estimate(r#"{"hours": 3.5, "reasoning": "Small change."}"#).await;
    assert_eq!(estimate.status, ROIEstimateStatus::Estimated);
    assert_hours(&estimate, 3.5_f64);
    assert_eq!(estimate.reasoning, "Small change.");
    assert_eq!(estimate.model.as_deref(), Some("gpt-5.2"));
    assert_eq!(estimate.evidence_source.as_deref(), Some("pr_metadata"));
    assert_eq!(estimate.effort_basis.as_deref(), Some("without_ai"));
    assert_eq!(estimate.cached, Some(false));
}

#[tokio::test]
async fn estimate_truncates_reasoning_to_twelve_thousand_characters() {
    let content = content_with_reasoning(&"\u{e9}".repeat(REASONING_MAX_CHARS + 1));
    let estimate = run_estimate(&content).await;
    assert_eq!(estimate.reasoning, "\u{e9}".repeat(REASONING_MAX_CHARS));
}

#[tokio::test]
async fn estimate_keeps_reasoning_at_exactly_twelve_thousand_characters() {
    let content = content_with_reasoning(&"\u{e9}".repeat(REASONING_MAX_CHARS));
    let estimate = run_estimate(&content).await;
    assert_eq!(estimate.reasoning, "\u{e9}".repeat(REASONING_MAX_CHARS));
}

#[tokio::test]
async fn estimate_renders_integer_hours_as_a_float() {
    let estimate = run_estimate(r#"{"hours": 3, "reasoning": "Small change."}"#).await;
    assert_hours(&estimate, 3.0_f64);
    assert!(
        serde_json::to_string(&estimate)
            .unwrap()
            .contains("\"hours\":3.0")
    );
}
