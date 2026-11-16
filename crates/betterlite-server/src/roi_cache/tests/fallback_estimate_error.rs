#![cfg(test)]

use betterlite_roi::records::ROIEstimateStatus;

use crate::roi_cache::fallback::estimate_with_fallback;
use crate::roi_estimator::Estimator;

use super::support::{NoCapabilities, answering_caller, evidence, failing_caller, settings};

const ESTIMATOR_FAILURE: &str =
    "The estimator did not return valid hours and reasoning. Check the selected model and prompt.";

#[tokio::test]
async fn an_estimator_failure_becomes_an_error_estimate() {
    let estimator = Estimator::new(settings(), failing_caller(), None);
    let mut evidence = evidence();
    evidence.incomplete_metadata = false;
    let estimate = estimate_with_fallback(&estimator, &evidence, &mut NoCapabilities).await;
    assert_eq!(estimate.status, ROIEstimateStatus::Error);
    assert_eq!(estimate.hours, None);
    assert_eq!(estimate.reasoning, ESTIMATOR_FAILURE);
    assert_eq!(estimate.model, None);
    assert_eq!(estimate.evidence_source, None);
    assert_eq!(estimate.effort_basis, None);
    assert_eq!(estimate.cached, None);
}

#[tokio::test]
async fn a_successful_estimate_passes_through() {
    let content = r#"{"hours": 3.5, "reasoning": "Small change."}"#;
    let estimator = Estimator::new(settings(), answering_caller(content), None);
    let mut evidence = evidence();
    evidence.incomplete_metadata = false;
    let estimate = estimate_with_fallback(&estimator, &evidence, &mut NoCapabilities).await;
    assert_eq!(estimate.status, ROIEstimateStatus::Estimated);
    assert_eq!(estimate.hours, Some(3.5_f64));
    assert_eq!(estimate.reasoning, "Small change.");
    assert_eq!(estimate.model.as_deref(), Some("gpt-5.2"));
}

#[tokio::test]
async fn a_needs_review_estimate_passes_through_without_calling_the_estimator() {
    let estimator = Estimator::new(settings(), failing_caller(), None);
    let estimate = estimate_with_fallback(&estimator, &evidence(), &mut NoCapabilities).await;
    assert_eq!(estimate.status, ROIEstimateStatus::NeedsReview);
    assert_eq!(estimate.hours, None);
}
