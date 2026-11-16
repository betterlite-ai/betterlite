#![cfg(test)]

use betterlite_roi::records::ROISyncPhase;

use super::support::{failing_factory, inputs, manager, manager_from, settled};
use crate::roi_cache::store::REPORT_PARAM;
use crate::roi_github::error::SourceError;

const DETAIL: &str = "/repos/o/r/pulls/12";

#[tokio::test]
async fn a_factory_error_fails_the_run_before_any_read() {
    let manager = manager_from(failing_factory());
    let inputs = inputs("o/r", &[12]);
    assert!(manager.start(inputs.run()).await);
    settled(&manager).await;
    let expected = SourceError::PaginationLimit.to_string();
    let status = manager.status();
    assert_eq!(status.phase, ROISyncPhase::Error);
    assert_eq!(status.stage, "Sync failed");
    assert_eq!(status.error.as_deref(), Some(expected.as_str()));
    assert!(!status.running);
    assert!(status.finished_at.is_some());
    assert!(inputs.spend.ranges().is_empty());
    assert!(inputs.repository.written_named(REPORT_PARAM).is_none());
}

#[tokio::test]
async fn a_pull_source_error_does_not_fail_the_run() {
    let manager = manager(None);
    let mut inputs = inputs("o/r", &[12, 13]);
    inputs.routes = inputs.routes.clone().with(DETAIL, 500, "{}");
    assert!(manager.start(inputs.run()).await);
    settled(&manager).await;
    let status = manager.status();
    assert_eq!(status.phase, ROISyncPhase::Complete);
    assert!(status.error.is_none());
    assert_eq!(status.needs_attention, 1);
    assert_eq!(status.estimated, 1);
}
