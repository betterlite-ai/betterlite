#![cfg(test)]

use std::sync::Arc;
use std::time::Duration;

use betterlite_roi::records::ROISyncPhase;
use tokio::sync::Notify;
use tokio::time::timeout;
use uuid::Uuid;

use super::support::{
    FakeCoordinator, Lease, TestClock, blank_status, drained, inputs, manager, manager_with_id,
    settings, settled, stalled_caller, until,
};
use crate::roi_sync::ports::UuidSource;

#[tokio::test]
async fn start_refuses_settings_without_repositories() {
    let manager = manager(None);
    let mut inputs = inputs("o/r", &[12]);
    inputs.settings = settings(&[]);
    assert!(!manager.start(inputs.run()).await);
    assert_eq!(manager.status().phase, ROISyncPhase::Idle);
    assert!(inputs.routes.requested().is_empty());
    assert!(inputs.spend.ranges().is_empty());
}

#[tokio::test]
async fn start_refuses_settings_without_an_estimator_model() {
    let manager = manager(None);
    let mut inputs = inputs("o/r", &[12]);
    inputs.settings.estimator_model = String::new();
    assert!(!manager.start(inputs.run()).await);
    assert_eq!(manager.status().phase, ROISyncPhase::Idle);
    assert!(inputs.routes.requested().is_empty());
}

#[tokio::test]
async fn start_reports_false_when_the_coordinator_refuses_the_lease() {
    let manager = manager(None);
    let mut inputs = inputs("o/r", &[12]);
    let coordinator = FakeCoordinator::default();
    coordinator.refusing(Lease::Acquire);
    inputs.coordinator = Some(coordinator.clone());
    inputs.interval = 5.5;
    assert!(!manager.start(inputs.run()).await);
    let acquires = coordinator.acquires();
    assert_eq!(acquires.len(), 1);
    assert!(Uuid::parse_str(&acquires[0].0).is_ok());
    assert!(acquires[0].1.running);
    assert_eq!(acquires[0].1.phase, ROISyncPhase::Spend);
    assert_eq!(acquires[0].1.stage, "Reading gateway spend");
    assert!(acquires[0].1.started_at.is_some());
    assert_eq!(acquires[0].2.to_bits(), 5.5_f64.to_bits());
    assert_eq!(manager.status().phase, ROISyncPhase::Idle);
    assert!(inputs.spend.ranges().is_empty());
}

#[tokio::test]
async fn start_spawns_the_analysis_after_a_successful_acquire() {
    let clock = TestClock::at("2026-01-10T00:00:00Z");
    let manager = manager(Some(clock.handle()));
    let mut inputs = inputs("o/r", &[12]);
    let coordinator = FakeCoordinator::default();
    inputs.coordinator = Some(coordinator.clone());
    inputs.interval = 5.5;
    assert!(manager.start(inputs.run()).await);
    let status = manager.status();
    assert!(status.running);
    assert_eq!(status.phase, ROISyncPhase::Spend);
    assert_eq!(
        status.started_at.as_deref(),
        Some("2026-01-10T00:00:00+00:00")
    );
    settled(&manager).await;
    assert_eq!(manager.status().phase, ROISyncPhase::Complete);
    let acquires = coordinator.acquires();
    assert_eq!(acquires.len(), 1);
    assert_eq!(
        acquires[0].1.started_at.as_deref(),
        Some("2026-01-10T00:00:00+00:00")
    );
    assert_eq!(acquires[0].1.done, 0);
    assert_eq!(acquires[0].2.to_bits(), 5.5_f64.to_bits());
    let finishes = coordinator.finishes();
    assert_eq!(finishes.len(), 1);
    assert_eq!(finishes[0].0, acquires[0].0);
}

#[tokio::test]
async fn start_refuses_a_second_run_while_one_is_stalled() {
    let manager = manager(None);
    let mut inputs = inputs("o/r", &[12]);
    inputs.complete = stalled_caller(Arc::new(Notify::new()));
    assert!(manager.start(inputs.run()).await);
    drained().await;
    let refused = timeout(Duration::from_secs(5), manager.start(inputs.run())).await;
    assert_eq!(refused.ok(), Some(false));
    assert!(manager.status().running);
    assert!(manager.cancel().await);
}

#[tokio::test]
async fn start_cancels_a_stale_run_before_starting_again() {
    let clock = TestClock::at("2026-01-10T00:00:00Z");
    let manager = manager(Some(clock.handle()));
    let mut inputs = inputs("o/r", &[12]);
    let coordinator = FakeCoordinator::default();
    inputs.coordinator = Some(coordinator.clone());
    inputs.complete = stalled_caller(Arc::new(Notify::new()));
    assert!(manager.start(inputs.run()).await);
    drained().await;
    clock.set("2026-01-10T01:00:00Z");
    assert!(manager.start(inputs.run()).await);
    assert_eq!(coordinator.acquires().len(), 2);
    let finishes = coordinator.finishes();
    assert_eq!(finishes.len(), 2);
    let phases: Vec<ROISyncPhase> = finishes.iter().map(|(_, status, _)| status.phase).collect();
    assert_eq!(
        phases,
        vec![ROISyncPhase::Cancelled, ROISyncPhase::Cancelled]
    );
    assert!(finishes.iter().all(|(_, _, report)| report.is_none()));
    let status = manager.status();
    assert!(status.running);
    assert_eq!(status.phase, ROISyncPhase::Spend);
    assert_eq!(
        status.started_at.as_deref(),
        Some("2026-01-10T01:00:00+00:00")
    );
    assert!(manager.cancel().await);
}

#[tokio::test]
async fn start_refuses_when_the_shared_run_is_still_running() {
    let manager = manager(None);
    let mut inputs = inputs("o/r", &[12]);
    let coordinator = FakeCoordinator::default();
    inputs.coordinator = Some(coordinator.clone());
    inputs.complete = stalled_caller(Arc::new(Notify::new()));
    assert!(manager.start(inputs.run()).await);
    drained().await;
    let mut shared = blank_status();
    shared.running = true;
    shared.phase = ROISyncPhase::Estimates;
    coordinator.sharing(shared);
    assert!(!manager.start(inputs.run()).await);
    assert_eq!(coordinator.acquires().len(), 1);
    assert!(coordinator.finishes().is_empty());
    assert!(manager.status().running);
    assert!(manager.cancel().await);
}

#[tokio::test]
async fn start_leaves_a_running_analysis_when_the_new_settings_are_invalid() {
    let manager = manager(None);
    let mut first = inputs("o/r", &[12]);
    first.complete = stalled_caller(Arc::new(Notify::new()));
    assert!(manager.start(first.run()).await);
    drained().await;
    let mut invalid = inputs("o/r", &[12]);
    invalid.settings = settings(&[]);
    assert!(!manager.start(invalid.run()).await);
    assert!(manager.status().running);
    assert!(manager.cancel().await);
}

#[tokio::test]
async fn start_acquires_the_lease_with_the_injected_owner_id() {
    let owner = "6f0b6a2e-1c55-4a5e-9a10-2f0d3c4b5a69";
    let next_id: UuidSource = Arc::new(move || Uuid::parse_str(owner).unwrap());
    let manager = manager_with_id(next_id);
    let mut inputs = inputs("o/r", &[12]);
    let coordinator = FakeCoordinator::default();
    inputs.coordinator = Some(coordinator.clone());
    assert!(manager.start(inputs.run()).await);
    settled(&manager).await;
    assert_eq!(coordinator.acquires()[0].0, owner);
    assert_eq!(coordinator.finishes()[0].0, owner);
}

#[tokio::test]
async fn start_serializes_concurrent_starts() {
    let manager = Arc::new(manager(None));
    let coordinator = FakeCoordinator::default();
    let gate = Arc::new(Notify::new());
    coordinator.holding_acquires(Arc::clone(&gate));
    let mut shared = blank_status();
    shared.running = true;
    shared.phase = ROISyncPhase::Estimates;
    coordinator.sharing(shared);
    let mut first = inputs("o/r", &[12]);
    first.coordinator = Some(coordinator.clone());
    first.complete = stalled_caller(Arc::new(Notify::new()));
    let mut second = inputs("o/r", &[12]);
    second.coordinator = Some(coordinator.clone());
    let release = async {
        until(|| coordinator.acquires().len() >= 1).await;
        drained().await;
        gate.notify_one();
    };
    let joined = timeout(Duration::from_secs(5), async {
        tokio::join!(
            manager.start(first.run()),
            manager.start(second.run()),
            release
        )
    })
    .await
    .expect("both starts finished");
    let (one, two, ()) = joined;
    assert_ne!(one, two);
    assert_eq!(coordinator.acquires().len(), 1);
    assert!(coordinator.finishes().is_empty());
    assert!(manager.status().running);
    assert!(manager.cancel().await);
}
