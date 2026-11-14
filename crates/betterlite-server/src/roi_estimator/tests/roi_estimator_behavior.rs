#![cfg(test)]

use betterlite_roi::estimator::models::ReasoningEffort;
use betterlite_roi::records::ROIEstimateStatus;
use rstest::rstest;

use crate::roi_estimator::Estimator;
use crate::roi_estimator::estimate::{INCOMPLETE_METADATA_REASON, OVERSIZED_EVIDENCE_REASON};

use super::support::{
    ModelSupport, OK_CONTENT, assert_hours, caller, ok_response, pull, recorded_request, settings,
};

#[rstest]
#[case::supported_models(&["gpt-5.2"], Some(ReasoningEffort::None))]
#[case::unsupported_models(&[], None)]
#[tokio::test]
async fn roi_estimator_behavior(
    #[case] supported: &[&str],
    #[case] effort: Option<ReasoningEffort>,
) {
    let (complete, requests) = caller(Ok(ok_response(OK_CONTENT)));
    let estimator = Estimator::new(settings("gpt-5.2", "Estimate this."), complete, None);
    let mut predicates = ModelSupport::supporting(supported);
    let estimate = estimator.estimate(&pull(), &mut predicates).await.unwrap();
    assert_eq!(estimate.status, ROIEstimateStatus::Estimated);
    assert_hours(&estimate, 3.5_f64);
    assert_eq!(recorded_request(&requests).reasoning_effort, effort);
}

#[rstest]
#[case::incomplete(false, true, INCOMPLETE_METADATA_REASON)]
#[case::oversized(true, false, OVERSIZED_EVIDENCE_REASON)]
#[tokio::test]
async fn roi_estimator_behavior_needs_review(
    #[case] oversized: bool,
    #[case] incomplete: bool,
    #[case] reasoning: &str,
) {
    let mut pull = pull();
    pull.incomplete_metadata = incomplete;
    if oversized {
        pull.body = "x".repeat(200_000);
    }
    let (complete, requests) = caller(Ok(ok_response(OK_CONTENT)));
    let estimator = Estimator::new(settings("gpt-5.2", "Estimate this."), complete, None);
    let estimate = estimator
        .estimate(&pull, &mut ModelSupport::none())
        .await
        .unwrap();
    assert_eq!(estimate.status, ROIEstimateStatus::NeedsReview);
    assert!(estimate.hours.is_none());
    assert_eq!(estimate.reasoning, reasoning);
    assert!(requests.lock().unwrap().is_empty());
}
