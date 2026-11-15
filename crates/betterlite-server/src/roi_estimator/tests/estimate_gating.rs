#![cfg(test)]

use betterlite_roi::estimator::evidence::metadata_evidence;
use betterlite_roi::estimator::json::json_dumps_text;
use betterlite_roi::estimator::models::MAX_EVIDENCE_CHARS;
use betterlite_roi::records::{ROIEstimateStatus, ROIPullEvidence};

use crate::roi_estimator::Estimator;
use crate::roi_estimator::estimate::{INCOMPLETE_METADATA_REASON, OVERSIZED_EVIDENCE_REASON};

use super::support::{
    ModelSupport, OK_CONTENT, caller, ok_response, pull, recorded_request, settings,
};

fn pull_with_body(length: usize) -> ROIPullEvidence {
    let mut value = pull();
    value.body = "\u{e9}".repeat(length);
    value
}

#[tokio::test]
async fn estimate_scores_evidence_at_exactly_the_character_limit() {
    let base = json_dumps_text(&metadata_evidence(&pull_with_body(0)))
        .unwrap()
        .chars()
        .count();
    let pull = pull_with_body(MAX_EVIDENCE_CHARS - base);
    assert_eq!(
        json_dumps_text(&metadata_evidence(&pull))
            .unwrap()
            .chars()
            .count(),
        MAX_EVIDENCE_CHARS
    );
    let (complete, requests) = caller(Ok(ok_response(OK_CONTENT)));
    let estimator = Estimator::new(settings("gpt-5.2", "Estimate this."), complete, Some(&[]));
    let estimate = estimator
        .estimate(&pull, &mut ModelSupport::none())
        .await
        .unwrap();
    assert!(
        json_dumps_text(&metadata_evidence(&pull)).unwrap().len() > MAX_EVIDENCE_CHARS,
        "the at-limit pull must exceed the limit in bytes so a byte count would refuse it"
    );
    assert_eq!(estimate.status, ROIEstimateStatus::Estimated);
    assert_eq!(recorded_request(&requests).model, "gpt-5.2");
}

#[tokio::test]
async fn estimate_refuses_one_character_over_the_limit_without_calling_the_model() {
    let base = json_dumps_text(&metadata_evidence(&pull_with_body(0)))
        .unwrap()
        .chars()
        .count();
    let pull = pull_with_body(MAX_EVIDENCE_CHARS - base + 1);
    let (complete, requests) = caller(Ok(ok_response(OK_CONTENT)));
    let estimator = Estimator::new(settings("gpt-5.2", "Estimate this."), complete, Some(&[]));
    let estimate = estimator
        .estimate(&pull, &mut ModelSupport::none())
        .await
        .unwrap();
    assert_eq!(estimate.status, ROIEstimateStatus::NeedsReview);
    assert!(estimate.hours.is_none());
    assert_eq!(estimate.reasoning, OVERSIZED_EVIDENCE_REASON);
    assert!(requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn estimate_refuses_pulls_with_incomplete_metadata_without_calling_the_model() {
    let mut pull = pull();
    pull.incomplete_metadata = true;
    let (complete, requests) = caller(Ok(ok_response(OK_CONTENT)));
    let estimator = Estimator::new(settings("gpt-5.2", "Estimate this."), complete, Some(&[]));
    let estimate = estimator
        .estimate(&pull, &mut ModelSupport::none())
        .await
        .unwrap();
    assert_eq!(estimate.status, ROIEstimateStatus::NeedsReview);
    assert!(estimate.hours.is_none());
    assert_eq!(estimate.reasoning, INCOMPLETE_METADATA_REASON);
    assert!(requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn estimate_reports_incomplete_metadata_before_oversized_evidence() {
    let mut pull = pull_with_body(MAX_EVIDENCE_CHARS + 1);
    pull.incomplete_metadata = true;
    let (complete, _requests) = caller(Ok(ok_response(OK_CONTENT)));
    let estimator = Estimator::new(settings("gpt-5.2", "Estimate this."), complete, Some(&[]));
    let estimate = estimator
        .estimate(&pull, &mut ModelSupport::none())
        .await
        .unwrap();
    assert_eq!(estimate.reasoning, INCOMPLETE_METADATA_REASON);
    assert_eq!(estimate.status, ROIEstimateStatus::NeedsReview);
}
