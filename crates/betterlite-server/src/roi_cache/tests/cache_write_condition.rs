#![cfg(test)]

use std::sync::Mutex;

use betterlite_roi::records::{ROIEstimateStatus, ROIPullRecord};
use rstest::rstest;

use crate::roi_cache::error::CacheWriteError;
use crate::roi_cache::store::cache_estimated_pull;

use super::support::{MemoryError, MemoryRepository, record};

fn change_profile(record: &mut ROIPullRecord) {
    record.profile_email = "new@example.com".to_owned();
}

fn change_emails(record: &mut ROIPullRecord) {
    record.emails = vec!["dev@example.com".to_owned(), "new@example.com".to_owned()];
}

#[tokio::test]
async fn a_new_estimated_pull_is_written_under_its_key() {
    let repository = MemoryRepository::default();
    let record = record(ROIEstimateStatus::Estimated);
    cache_estimated_pull(&repository, Some("key-1"), &record, None)
        .await
        .unwrap();
    let written = repository.written();
    assert_eq!(written.len(), 1);
    assert_eq!(written[0].0, "roi_calculator_pull_key-1");
    let stored: ROIPullRecord =
        serde_json::from_slice(&serde_json::to_vec(&written[0].1).unwrap()).unwrap();
    assert_eq!(stored, record);
}

#[rstest]
#[case::profile_email(change_profile)]
#[case::emails(change_emails)]
#[tokio::test]
async fn changed_identity_evidence_is_written_again(#[case] change: fn(&mut ROIPullRecord)) {
    let repository = MemoryRepository::default();
    let previous = record(ROIEstimateStatus::Estimated);
    let mut current = previous.clone();
    change(&mut current);
    cache_estimated_pull(&repository, Some("key-1"), &current, Some(&previous))
        .await
        .unwrap();
    assert_eq!(repository.written().len(), 1);
}

#[tokio::test]
async fn a_refused_write_is_reported_as_a_store_error() {
    let repository = MemoryRepository {
        params: Mutex::default(),
        writes: Mutex::default(),
        refuse_writes: true,
        refuse_reads: false,
    };
    let record = record(ROIEstimateStatus::Estimated);
    let error = cache_estimated_pull(&repository, Some("key-1"), &record, None)
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        CacheWriteError::Store(MemoryError::Refused)
    ));
}
