#![cfg(test)]

use betterlite_roi::records::ROIEstimateStatus;

use crate::roi_cache::records::report_record;

use super::support::{estimate, evidence, record};

#[test]
fn the_record_takes_the_evidence_estimate_and_key() {
    let built = report_record(
        &evidence(),
        estimate(ROIEstimateStatus::Estimated),
        Some("key-1".to_owned()),
    );
    assert_eq!(built, record(ROIEstimateStatus::Estimated));
}

#[test]
fn a_missing_key_stays_missing() {
    let built = report_record(&evidence(), estimate(ROIEstimateStatus::NeedsReview), None);
    assert_eq!(built.cache_key, None);
    assert_eq!(built.estimate, estimate(ROIEstimateStatus::NeedsReview));
}

#[test]
fn missing_commit_emails_become_an_empty_list() {
    let mut evidence = evidence();
    evidence.commit_emails = None;
    let built = report_record(&evidence, estimate(ROIEstimateStatus::Estimated), None);
    assert_eq!(built.commit_emails, Some(Vec::new()));
}
