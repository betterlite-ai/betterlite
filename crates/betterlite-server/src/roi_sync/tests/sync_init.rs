#![cfg(test)]

use betterlite_roi::records::ROISyncPhase;
use chrono::{DateTime, TimeDelta, Utc};

use super::support::{TestClock, inputs, manager, settled};

#[tokio::test]
async fn new_reports_an_idle_status_with_zeroed_counters() {
    let manager = manager(None);
    let status = manager.status();
    assert!(!status.running);
    assert_eq!(status.phase, ROISyncPhase::Idle);
    assert_eq!(status.stage, "Idle");
    assert_eq!(status.done, 0);
    assert_eq!(status.total, 0);
    assert_eq!(status.estimated, 0);
    assert_eq!(status.reused, 0);
    assert_eq!(status.needs_attention, 0);
    assert!(status.error.is_none());
    assert!(status.started_at.is_none());
    assert!(status.finished_at.is_none());
    assert!(status.next_update.is_none());
    assert_eq!(status.elapsed_seconds, 0);
    assert!(status.remaining_seconds.is_none());
}

#[tokio::test]
async fn new_keeps_each_manager_on_its_own_status() {
    let first = manager(None);
    let second = manager(None);
    let inputs = inputs("o/r", &[]);
    assert!(first.start(inputs.run()).await);
    settled(&first).await;
    assert_eq!(first.status().phase, ROISyncPhase::Complete);
    assert_eq!(second.status().phase, ROISyncPhase::Idle);
}

#[tokio::test]
async fn new_without_a_clock_stamps_the_system_time() {
    let before = Utc::now();
    let manager = manager(None);
    let inputs = inputs("o/r", &[]);
    assert!(manager.start(inputs.run()).await);
    let after = Utc::now();
    let started = manager.status().started_at.unwrap();
    let parsed = DateTime::parse_from_rfc3339(&started)
        .unwrap()
        .with_timezone(&Utc);
    assert!(parsed <= after);
    assert!(parsed >= before - TimeDelta::microseconds(1));
    settled(&manager).await;
}

#[tokio::test]
async fn new_with_a_clock_stamps_that_clock() {
    let clock = TestClock::at("2026-03-01T12:30:15Z");
    let manager = manager(Some(clock.handle()));
    let inputs = inputs("o/r", &[]);
    assert!(manager.start(inputs.run()).await);
    assert_eq!(
        manager.status().started_at.as_deref(),
        Some("2026-03-01T12:30:15+00:00")
    );
    settled(&manager).await;
}
