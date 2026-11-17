#![cfg(test)]

use std::sync::Arc;

use betterlite_roi::records::ROISyncPhase;
use tokio::sync::Notify;

use super::support::{FakeCoordinator, drained, inputs, manager, settled, stalled_caller};
use crate::roi_cache::store::REPORT_PARAM;

#[tokio::test]
async fn cancel_without_a_task_reports_false() {
    let manager = manager(None);
    assert!(!manager.cancel().await);
    assert_eq!(manager.status().phase, ROISyncPhase::Idle);
    assert!(!manager.status().running);
}

#[tokio::test]
async fn cancel_stops_the_analysis_and_marks_it_cancelled() {
    let manager = manager(None);
    let mut inputs = inputs("o/r", &[12]);
    inputs.complete = stalled_caller(Arc::new(Notify::new()));
    assert!(manager.start(inputs.run()).await);
    drained().await;
    assert!(manager.cancel().await);
    let status = manager.status();
    assert!(!status.running);
    assert_eq!(status.phase, ROISyncPhase::Cancelled);
    assert_eq!(status.stage, "Sync cancelled");
    assert!(status.finished_at.is_some());
    assert!(inputs.repository.written_named(REPORT_PARAM).is_none());
    assert!(!manager.cancel().await);
}

#[tokio::test]
async fn cancel_reports_false_after_the_analysis_completed() {
    let manager = manager(None);
    let inputs = inputs("o/r", &[12]);
    assert!(manager.start(inputs.run()).await);
    settled(&manager).await;
    assert!(!manager.cancel().await);
    let status = manager.status();
    assert_eq!(status.phase, ROISyncPhase::Complete);
    assert_eq!(status.stage, "Analysis complete");
}

#[tokio::test]
async fn cancel_notifies_the_coordinator_of_the_cancelled_status() {
    let manager = manager(None);
    let mut inputs = inputs("o/r", &[12]);
    let coordinator = FakeCoordinator::default();
    inputs.coordinator = Some(coordinator.clone());
    inputs.complete = stalled_caller(Arc::new(Notify::new()));
    assert!(manager.start(inputs.run()).await);
    drained().await;
    assert!(manager.cancel().await);
    let finishes = coordinator.finishes();
    assert_eq!(finishes.len(), 2);
    let owner = coordinator.acquires()[0].0.clone();
    assert!(finishes.iter().all(|(finished, _, _)| *finished == owner));
    assert!(finishes.iter().all(|(_, status, report)| {
        status.phase == ROISyncPhase::Cancelled && report.is_none()
    }));
}

#[tokio::test]
async fn cancel_returns_only_after_the_workers_stopped() {
    let manager = manager(None);
    let mut inputs = inputs("o/r", &[12, 13, 14]);
    inputs.complete = stalled_caller(Arc::new(Notify::new()));
    assert!(manager.start(inputs.run()).await);
    drained().await;
    assert!(manager.cancel().await);
    let seen = inputs.routes.requested().len();
    drained().await;
    assert_eq!(inputs.routes.requested().len(), seen);
    assert!(inputs.repository.written_named(REPORT_PARAM).is_none());
}
