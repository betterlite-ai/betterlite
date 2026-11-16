#![cfg(test)]

use std::sync::Arc;

use betterlite_roi::records::ROISyncPhase;
use tokio::sync::Notify;
use tokio::time::advance;

use super::support::{FakeCoordinator, drained, inputs, manager, settled, stalled_caller, until};
use crate::roi_cache::store::REPORT_PARAM;
use crate::roi_sync::heartbeat::HEARTBEAT_INTERVAL;

#[tokio::test(start_paused = true)]
async fn cancel_stops_the_heartbeat() {
    let manager = manager(None);
    let mut inputs = inputs("o/r", &[12]);
    let coordinator = FakeCoordinator::default();
    inputs.coordinator = Some(coordinator.clone());
    inputs.complete = stalled_caller(Arc::new(Notify::new()));
    assert!(manager.start(inputs.run()).await);
    drained().await;
    advance(HEARTBEAT_INTERVAL).await;
    until(|| coordinator.beats().len() == 1).await;
    assert!(manager.cancel().await);
    advance(HEARTBEAT_INTERVAL.saturating_mul(10)).await;
    drained().await;
    assert_eq!(coordinator.beats().len(), 1);
    assert_eq!(manager.status().phase, ROISyncPhase::Cancelled);
    assert!(inputs.repository.written_named(REPORT_PARAM).is_none());
}

#[tokio::test]
async fn cancelling_a_finished_run_leaves_the_coordinator_alone() {
    let manager = manager(None);
    let mut inputs = inputs("o/r", &[12]);
    let coordinator = FakeCoordinator::default();
    inputs.coordinator = Some(coordinator.clone());
    assert!(manager.start(inputs.run()).await);
    settled(&manager).await;
    let finishes = coordinator.finishes().len();
    assert!(!manager.cancel().await);
    assert_eq!(coordinator.finishes().len(), finishes);
    assert_eq!(manager.status().phase, ROISyncPhase::Complete);
    assert_eq!(manager.status().stage, "Analysis complete");
}
