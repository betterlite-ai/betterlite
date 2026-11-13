#![cfg(test)]

use betterlite_roi::records::ROIEstimateStatus;
use rstest::rstest;
use serde_json::json;

use crate::roi_estimator::Estimator;
use crate::roi_estimator::estimate::{INCOMPLETE_METADATA_REASON, OVERSIZED_EVIDENCE_REASON};

use super::support::{
    ModelSupport, OK_CONTENT, caller, ok_response, pull, recorded_request, settings,
};

#[rstest]
#[case::supported_models(&["gpt-5.2"], Some("none"))]
#[case::unsupported_models(&[], None)]
#[tokio::test]
async fn roi_estimator_behavior(#[case] supported: &[&str], #[case] effort: Option<&str>) {
    let (complete, requests) = caller(Ok(ok_response(OK_CONTENT)));
    let estimator = Estimator::new(settings("gpt-5.2", "Estimate this."), complete, None);
    let mut predicates = ModelSupport::supporting(supported);
    let estimate = estimator.estimate(&pull(), &mut predicates).await.unwrap();
    assert_eq!(estimate.status, ROIEstimateStatus::Estimated);
    assert_eq!(estimate.hours, Some(3.5));
    let request = serde_json::to_value(recorded_request(&requests)).unwrap();
    assert_eq!(request["reasoning_effort"], json!(effort));
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
    assert_eq!(
        serde_json::to_value(&estimate).unwrap(),
        json!({"status": "needs_review", "hours": null, "reasoning": reasoning})
    );
    assert!(requests.lock().unwrap().is_empty());
}
