#![cfg(test)]

use betterlite_roi::records::ROIEstimateStatus;
use rstest::rstest;

use crate::roi_cache::error::ReportBlocked;
use crate::roi_cache::processed::processed_records;

use super::support::processed;

const NO_METADATA: &str =
    "GitHub could not provide PR metadata. No new report was published; try analysis again later.";

#[rstest]
#[case::needs_review(ROIEstimateStatus::NeedsReview)]
#[case::error(ROIEstimateStatus::Error)]
#[case::estimated(ROIEstimateStatus::Estimated)]
fn pulls_with_no_readable_metadata_block_the_report(#[case] status: ROIEstimateStatus) {
    let batch = vec![processed(0, status, true), processed(1, status, true)];
    let error = processed_records(batch).unwrap_err();
    assert!(matches!(error, ReportBlocked::NoMetadata));
    assert_eq!(error.to_string(), NO_METADATA);
}

#[test]
fn one_readable_pull_keeps_the_report_possible() {
    let batch = vec![
        processed(0, ROIEstimateStatus::NeedsReview, true),
        processed(1, ROIEstimateStatus::Estimated, false),
    ];
    assert_eq!(processed_records(batch).unwrap().len(), 2);
}

#[test]
fn an_empty_batch_is_not_blocked() {
    assert!(processed_records(Vec::new()).unwrap().is_empty());
}
