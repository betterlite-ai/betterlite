#![cfg(test)]

use betterlite_roi::records::ROIEstimateStatus;
use rstest::rstest;

use crate::roi_cache::error::ReportBlocked;
use crate::roi_cache::processed::processed_records;

use super::support::processed;

const NO_ESTIMATES: &str = "The estimator could not score any pull requests. No new report was published; check the estimator connection or try analysis again later.";

#[rstest]
#[case::only_errors(&[ROIEstimateStatus::Error, ROIEstimateStatus::Error])]
#[case::errors_and_reviews(&[ROIEstimateStatus::NeedsReview, ROIEstimateStatus::Error])]
fn errors_without_any_estimate_block_the_report(#[case] statuses: &[ROIEstimateStatus]) {
    let batch = statuses
        .iter()
        .enumerate()
        .map(|(position, status)| processed(position, *status, false))
        .collect();
    let error = processed_records(batch).unwrap_err();
    assert!(matches!(error, ReportBlocked::NoEstimates));
    assert_eq!(error.to_string(), NO_ESTIMATES);
}

#[rstest]
#[case::error_and_estimate(&[ROIEstimateStatus::Error, ROIEstimateStatus::Estimated])]
#[case::only_reviews(&[ROIEstimateStatus::NeedsReview, ROIEstimateStatus::NeedsReview])]
#[case::only_estimates(&[ROIEstimateStatus::Estimated])]
fn a_batch_with_an_estimate_or_without_errors_is_accepted(#[case] statuses: &[ROIEstimateStatus]) {
    let batch = statuses
        .iter()
        .enumerate()
        .map(|(position, status)| processed(position, *status, false))
        .collect();
    assert_eq!(processed_records(batch).unwrap().len(), statuses.len());
}
