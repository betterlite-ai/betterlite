#![cfg(test)]

use betterlite_roi::records::{ROIEstimateStatus, ROISyncPhase};

use super::support::{inputs, manager, report_of, settled};
use crate::roi_cache::error::ReportBlocked;
use crate::roi_cache::store::REPORT_PARAM;

const DETAIL: &str = "/repos/o/r/pulls/12";

#[tokio::test]
async fn source_error_marks_the_pull_unavailable() {
    let manager = manager(None);
    let mut inputs = inputs("o/r", &[12, 13]);
    inputs.routes = inputs.routes.clone().with(DETAIL, 500, "{}");
    assert!(manager.start(inputs.run()).await);
    settled(&manager).await;
    let report = report_of(&inputs.repository);
    assert_eq!(report.pulls.len(), 2);
    let record = &report.pulls[0];
    assert_eq!(record.number, 12);
    assert!(record.incomplete_metadata);
    assert_eq!(record.additions, 0);
    assert_eq!(record.changed_files, 0);
    assert_eq!(record.commit_count, 0);
    assert!(record.cache_key.is_none());
    assert_eq!(record.estimate.status, ROIEstimateStatus::NeedsReview);
    assert!(
        record
            .estimate
            .reasoning
            .starts_with("PR metadata could not be read: ")
    );
    let status = manager.status();
    assert_eq!(status.phase, ROISyncPhase::Complete);
    assert_eq!(status.done, 2);
    assert_eq!(status.estimated, 1);
    assert_eq!(status.needs_attention, 1);
}

#[tokio::test]
async fn unavailable_metadata_without_a_profile_keeps_no_emails() {
    let manager = manager(None);
    let mut inputs = inputs("o/r", &[12, 13]);
    inputs.routes = inputs
        .routes
        .clone()
        .with(DETAIL, 500, "{}")
        .with("/users/dev", 404, "");
    assert!(manager.start(inputs.run()).await);
    settled(&manager).await;
    let report = report_of(&inputs.repository);
    let record = &report.pulls[0];
    assert!(record.profile_email.is_empty());
    assert!(record.emails.is_empty());
    assert_eq!(
        record.commit_emails,
        Some(Vec::<String>::new()),
        "the unavailable record carries an empty commit list"
    );
}

#[tokio::test]
async fn every_unavailable_pull_blocks_the_report() {
    let manager = manager(None);
    let mut inputs = inputs("o/r", &[12]);
    inputs.routes = inputs.routes.clone().with(DETAIL, 500, "{}");
    assert!(manager.start(inputs.run()).await);
    settled(&manager).await;
    let expected = ReportBlocked::NoMetadata.to_string();
    let status = manager.status();
    assert_eq!(status.phase, ROISyncPhase::Error);
    assert_eq!(status.error.as_deref(), Some(expected.as_str()));
    assert_eq!(status.done, 1);
    assert_eq!(status.needs_attention, 1);
    assert!(inputs.repository.written_named(REPORT_PARAM).is_none());
}
