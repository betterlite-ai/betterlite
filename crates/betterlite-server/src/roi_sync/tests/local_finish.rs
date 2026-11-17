#![cfg(test)]

use betterlite_roi::records::ROISyncPhase;

use super::support::{inputs, manager, report_of, settled};
use crate::roi_cache::store::REPORT_PARAM;

#[tokio::test]
async fn a_local_run_completes_and_stores_its_report_once() {
    let manager = manager(None);
    let inputs = inputs("o/r", &[12, 13]);
    assert!(manager.start(inputs.run()).await);
    settled(&manager).await;
    let status = manager.status();
    assert_eq!(status.phase, ROISyncPhase::Complete);
    assert_eq!(status.stage, "Analysis complete");
    assert!(!status.running);
    assert!(status.finished_at.is_some());
    let reports = inputs
        .repository
        .written()
        .iter()
        .filter(|entry| entry.0 == REPORT_PARAM)
        .count();
    assert_eq!(reports, 1);
    assert_eq!(report_of(&inputs.repository).pulls.len(), 2);
}

#[tokio::test]
async fn a_local_run_without_merged_pulls_still_completes() {
    let manager = manager(None);
    let inputs = inputs("o/r", &[]);
    assert!(manager.start(inputs.run()).await);
    settled(&manager).await;
    let status = manager.status();
    assert_eq!(status.phase, ROISyncPhase::Complete);
    assert_eq!(status.total, 0);
    assert_eq!(status.done, 0);
    assert!(report_of(&inputs.repository).pulls.is_empty());
}
