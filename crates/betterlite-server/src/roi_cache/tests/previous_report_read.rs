#![cfg(test)]

use std::sync::Mutex;

use rstest::rstest;

use crate::roi_cache::store::previous_report;

use super::support::{MemoryError, MemoryRepository, report};

#[tokio::test]
async fn a_stored_report_is_read_back() {
    let repository = MemoryRepository::holding("roi_calculator_report", &report());
    assert_eq!(previous_report(&repository).await.unwrap(), Some(report()));
}

#[tokio::test]
async fn a_missing_report_reads_as_none() {
    let repository = MemoryRepository::holding("roi_calculator_reports", &report());
    assert_eq!(previous_report(&repository).await.unwrap(), None);
}

#[tokio::test]
async fn a_failing_read_is_an_error_not_a_missing_report() {
    let repository = MemoryRepository {
        params: Mutex::default(),
        writes: Mutex::default(),
        refuse_writes: false,
        refuse_reads: true,
    };
    let error = previous_report(&repository).await.unwrap_err();
    assert!(matches!(error, MemoryError::Unreadable));
}

#[rstest]
#[case::empty_object("{}")]
#[case::wrong_shape(r#"{"mode": 3}"#)]
#[case::a_string(r#""report""#)]
#[tokio::test]
async fn a_report_that_fails_validation_reads_as_none(#[case] stored: &str) {
    let repository = MemoryRepository::holding_text("roi_calculator_report", stored);
    assert_eq!(previous_report(&repository).await.unwrap(), None);
}
