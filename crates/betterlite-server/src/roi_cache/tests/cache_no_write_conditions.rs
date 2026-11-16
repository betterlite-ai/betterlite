#![cfg(test)]

use betterlite_roi::records::ROIEstimateStatus;
use rstest::rstest;

use crate::roi_cache::store::cache_estimated_pull;

use super::support::{MemoryRepository, record};

#[rstest]
#[case::needs_review(ROIEstimateStatus::NeedsReview)]
#[case::error(ROIEstimateStatus::Error)]
#[tokio::test]
async fn an_estimate_that_did_not_succeed_is_not_written(#[case] status: ROIEstimateStatus) {
    let repository = MemoryRepository::default();
    let record = record(status);
    cache_estimated_pull(&repository, Some("key-1"), &record, None)
        .await
        .unwrap();
    assert!(repository.written().is_empty());
}

#[tokio::test]
async fn a_pull_without_a_key_is_not_written() {
    let repository = MemoryRepository::default();
    let record = record(ROIEstimateStatus::Estimated);
    cache_estimated_pull(&repository, None, &record, None)
        .await
        .unwrap();
    assert!(repository.written().is_empty());
}

#[tokio::test]
async fn unchanged_profile_and_emails_are_not_written_again() {
    let repository = MemoryRepository::default();
    let previous = record(ROIEstimateStatus::Estimated);
    let mut current = previous.clone();
    current.commit_emails = Some(vec!["other@example.com".to_owned()]);
    current.estimate.hours = Some(9.0_f64);
    cache_estimated_pull(&repository, Some("key-1"), &current, Some(&previous))
        .await
        .unwrap();
    assert!(repository.written().is_empty());
}
