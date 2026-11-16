#![cfg(test)]

use std::sync::Arc;

use betterlite_roi::records::ROISyncPhase;
use tokio::sync::Notify;
use tokio::time::advance;

use super::support::{
    FakeCoordinator, Lease, TestClock, drained, inputs, manager, settled, stalled_caller, until,
};
use crate::roi_cache::store::REPORT_PARAM;
use crate::roi_sync::heartbeat::HEARTBEAT_INTERVAL;

#[tokio::test(start_paused = true)]
async fn heartbeat_renews_the_lease_every_second() {
    let clock = TestClock::at("2026-01-10T00:00:00Z");
    let manager = manager(Some(clock.handle()));
    let mut inputs = inputs("o/r", &[12]);
    let coordinator = FakeCoordinator::default();
    inputs.coordinator = Some(coordinator.clone());
    inputs.complete = stalled_caller(Arc::new(Notify::new()));
    assert!(manager.start(inputs.run()).await);
    drained().await;
    advance(HEARTBEAT_INTERVAL).await;
    until(|| coordinator.beats().len() == 1).await;
    advance(HEARTBEAT_INTERVAL).await;
    until(|| coordinator.beats().len() == 2).await;
    let beats = coordinator.beats();
    assert_eq!(beats[0].0, coordinator.acquires()[0].0);
    assert!(beats[0].1.running);
    assert_eq!(beats[0].1.phase, ROISyncPhase::Estimates);
    assert!(manager.cancel().await);
}

#[tokio::test(start_paused = true)]
async fn heartbeat_cancels_the_run_when_the_lease_is_lost() {
    let manager = manager(None);
    let mut inputs = inputs("o/r", &[12]);
    let coordinator = FakeCoordinator::default();
    coordinator.refusing(Lease::Heartbeat);
    inputs.coordinator = Some(coordinator.clone());
    inputs.complete = stalled_caller(Arc::new(Notify::new()));
    assert!(manager.start(inputs.run()).await);
    drained().await;
    advance(HEARTBEAT_INTERVAL).await;
    settled(&manager).await;
    let status = manager.status();
    assert!(!status.running);
    assert_eq!(status.phase, ROISyncPhase::Cancelled);
    assert_eq!(status.stage, "Sync cancelled");
    assert!(status.finished_at.is_some());
    assert!(inputs.repository.written_named(REPORT_PARAM).is_none());
    assert_eq!(coordinator.beats().len(), 1);
    let finishes = coordinator.finishes();
    assert_eq!(finishes.len(), 1);
    assert_eq!(finishes[0].1.phase, ROISyncPhase::Cancelled);
    assert!(finishes[0].2.is_none());
}

#[tokio::test(start_paused = true)]
async fn heartbeat_does_not_run_without_a_coordinator() {
    let manager = manager(None);
    let mut inputs = inputs("o/r", &[12]);
    inputs.complete = stalled_caller(Arc::new(Notify::new()));
    assert!(manager.start(inputs.run()).await);
    drained().await;
    advance(HEARTBEAT_INTERVAL.saturating_mul(5)).await;
    drained().await;
    assert!(manager.status().running);
    assert!(manager.cancel().await);
}

#[tokio::test(start_paused = true)]
async fn heartbeat_stops_after_the_run_finishes() {
    let manager = manager(None);
    let mut inputs = inputs("o/r", &[12]);
    let coordinator = FakeCoordinator::default();
    inputs.coordinator = Some(coordinator.clone());
    assert!(manager.start(inputs.run()).await);
    settled(&manager).await;
    assert_eq!(manager.status().phase, ROISyncPhase::Complete);
    let beats = coordinator.beats().len();
    advance(HEARTBEAT_INTERVAL.saturating_mul(10)).await;
    drained().await;
    assert_eq!(coordinator.beats().len(), beats);
}
