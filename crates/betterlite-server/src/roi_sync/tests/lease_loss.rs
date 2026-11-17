#![cfg(test)]

use std::sync::Arc;

use betterlite_roi::records::ROISyncPhase;
use tokio::sync::Notify;
use tokio::time::advance;

use super::support::{
    FakeCoordinator, Inputs, Lease, drained, inputs, manager, settled, stalled_caller,
};
use crate::roi_cache::store::REPORT_PARAM;
use crate::roi_sync::heartbeat::HEARTBEAT_INTERVAL;

fn stalled_run(coordinator: &FakeCoordinator) -> Inputs {
    let mut inputs = inputs("o/r", &[12]);
    inputs.coordinator = Some(coordinator.clone());
    inputs.complete = stalled_caller(Arc::new(Notify::new()));
    inputs
}

#[tokio::test(start_paused = true)]
async fn lease_loss_cancels_the_run_without_publishing() {
    let manager = manager(None);
    let coordinator = FakeCoordinator::default();
    coordinator.refusing(Lease::Heartbeat);
    let inputs = stalled_run(&coordinator);
    assert!(manager.start(inputs.run()).await);
    drained().await;
    advance(HEARTBEAT_INTERVAL).await;
    settled(&manager).await;
    let status = manager.status();
    assert_eq!(status.phase, ROISyncPhase::Cancelled);
    assert_eq!(status.stage, "Sync cancelled");
    assert!(!status.running);
    assert!(inputs.repository.written_named(REPORT_PARAM).is_none());
}

#[tokio::test(start_paused = true)]
async fn lease_loss_notifies_the_coordinator_of_the_cancelled_run() {
    let manager = manager(None);
    let coordinator = FakeCoordinator::default();
    coordinator.refusing(Lease::Heartbeat);
    let inputs = stalled_run(&coordinator);
    assert!(manager.start(inputs.run()).await);
    drained().await;
    advance(HEARTBEAT_INTERVAL).await;
    settled(&manager).await;
    let finishes = coordinator.finishes();
    assert_eq!(finishes.len(), 1);
    assert_eq!(finishes[0].0, coordinator.acquires()[0].0);
    assert_eq!(finishes[0].1.phase, ROISyncPhase::Cancelled);
    assert!(finishes[0].2.is_none());
    assert!(inputs.repository.written_named(REPORT_PARAM).is_none());
}
