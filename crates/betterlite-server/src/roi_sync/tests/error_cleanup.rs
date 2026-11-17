#![cfg(test)]

use betterlite_roi::records::ROISyncPhase;
use tokio::time::advance;

use super::support::{FakeCoordinator, FakeSpend, drained, inputs, manager, settled};
use crate::roi_cache::store::REPORT_PARAM;
use crate::roi_sync::heartbeat::HEARTBEAT_INTERVAL;

#[tokio::test(start_paused = true)]
async fn a_failed_run_stops_the_heartbeat() {
    let manager = manager(None);
    let mut inputs = inputs("o/r", &[12]);
    inputs.spend = FakeSpend::failing();
    let coordinator = FakeCoordinator::default();
    inputs.coordinator = Some(coordinator.clone());
    assert!(manager.start(inputs.run()).await);
    settled(&manager).await;
    let beats = coordinator.beats().len();
    advance(HEARTBEAT_INTERVAL.saturating_mul(10)).await;
    drained().await;
    assert_eq!(coordinator.beats().len(), beats);
    assert_eq!(manager.status().phase, ROISyncPhase::Error);
}

#[tokio::test]
async fn a_failed_run_notifies_the_coordinator_without_a_report() {
    let manager = manager(None);
    let mut inputs = inputs("o/r", &[12]);
    inputs.spend = FakeSpend::failing();
    let coordinator = FakeCoordinator::default();
    inputs.coordinator = Some(coordinator.clone());
    assert!(manager.start(inputs.run()).await);
    settled(&manager).await;
    let finishes = coordinator.finishes();
    assert_eq!(finishes.len(), 1);
    assert_eq!(finishes[0].1.phase, ROISyncPhase::Error);
    assert_eq!(finishes[0].1.stage, "Sync failed");
    assert!(finishes[0].1.finished_at.is_some());
    assert!(finishes[0].2.is_none());
    assert!(inputs.repository.written_named(REPORT_PARAM).is_none());
}
