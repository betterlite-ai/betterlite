#![cfg(test)]

use betterlite_roi::records::ROISyncPhase;

use super::support::{FakeCoordinator, Lease, TestClock, inputs, manager, settled, until};
use crate::roi_cache::store::REPORT_PARAM;
use crate::roi_sync::error::REPLACED_MESSAGE;

const PULL_DETAIL: &str = "/repos/o/r/pulls/12";

#[tokio::test]
async fn coordinator_receives_the_completed_status_and_report() {
    let clock = TestClock::at("2026-01-10T00:00:00Z");
    let manager = manager(Some(clock.handle()));
    let mut inputs = inputs("o/r", &[12, 13, 14]);
    let coordinator = FakeCoordinator::default();
    inputs.coordinator = Some(coordinator.clone());
    let gate = inputs.routes.gate(PULL_DETAIL);
    assert!(manager.start(inputs.run()).await);
    until(|| {
        inputs
            .routes
            .requested()
            .iter()
            .any(|path| path == PULL_DETAIL)
    })
    .await;
    clock.set("2026-01-10T02:00:00Z");
    gate.notify_one();
    settled(&manager).await;
    let finishes = coordinator.finishes();
    assert_eq!(finishes.len(), 1);
    let (owner, status, completed) = &finishes[0];
    assert_eq!(*owner, coordinator.acquires()[0].0);
    assert_eq!(status.phase, ROISyncPhase::Complete);
    assert!(!status.running);
    assert_eq!(status.stage, "Analysis complete");
    assert_eq!(
        status.finished_at.as_deref(),
        Some("2026-01-10T02:00:00+00:00")
    );
    assert_eq!(status.elapsed_seconds, 7200);
    assert_eq!(status.remaining_seconds, Some(0));
    assert_eq!(status.done, 3);
    let published = completed
        .as_ref()
        .expect("the completed run hands its report to the coordinator");
    assert_eq!(published.pulls.len(), 3);
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
