#![cfg(test)]

use betterlite_roi::records::ROISyncPhase;

use super::support::{FakeCoordinator, Lease, inputs, manager, settled};
use crate::roi_cache::store::REPORT_PARAM;
use crate::roi_sync::error::REPLACED_MESSAGE;

#[tokio::test]
async fn coordinator_receives_the_completed_status_and_report() {
    let manager = manager(None);
    let mut inputs = inputs("o/r", &[12, 13]);
    let coordinator = FakeCoordinator::default();
    inputs.coordinator = Some(coordinator.clone());
    assert!(manager.start(inputs.run()).await);
    settled(&manager).await;
    let finishes = coordinator.finishes();
    assert_eq!(finishes.len(), 1);
    let (owner, status, completed) = &finishes[0];
    assert_eq!(*owner, coordinator.acquires()[0].0);
    assert_eq!(status.phase, ROISyncPhase::Complete);
    assert!(!status.running);
    assert_eq!(status.stage, "Analysis complete");
    assert!(status.finished_at.is_some());
    assert_eq!(status.done, 2);
    let published = completed
        .as_ref()
        .expect("the completed run hands its report to the coordinator");
    assert_eq!(published.pulls.len(), 2);
    assert!(inputs.repository.written_named(REPORT_PARAM).is_none());
}

#[tokio::test]
async fn a_replaced_publication_fails_the_run() {
    let manager = manager(None);
    let mut inputs = inputs("o/r", &[12]);
    let coordinator = FakeCoordinator::default();
    coordinator.refusing(Lease::Finish);
    inputs.coordinator = Some(coordinator.clone());
    assert!(manager.start(inputs.run()).await);
    settled(&manager).await;
    let status = manager.status();
    assert_eq!(status.phase, ROISyncPhase::Error);
    assert_eq!(status.stage, "Sync failed");
    assert_eq!(status.error.as_deref(), Some(REPLACED_MESSAGE));
    assert!(inputs.repository.written_named(REPORT_PARAM).is_none());
    let finishes = coordinator.finishes();
    assert_eq!(finishes.len(), 2);
    assert_eq!(finishes[0].1.phase, ROISyncPhase::Complete);
    assert!(finishes[0].2.is_some());
    assert_eq!(finishes[1].1.phase, ROISyncPhase::Error);
    assert!(finishes[1].2.is_none());
}
