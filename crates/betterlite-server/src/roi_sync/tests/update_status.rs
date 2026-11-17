#![cfg(test)]

use std::sync::{Arc, Mutex};

use betterlite_roi::records::{ROISyncPhase, ROISyncStatus};

use super::support::{TestClock, blank_status};
use crate::roi_sync::status::{StatusChange, StatusReporter};

const START: &str = "2026-01-10T00:00:00+00:00";

fn reporter(stored: ROISyncStatus, clock: &TestClock) -> StatusReporter {
    StatusReporter {
        stored: Arc::new(Mutex::new(stored)),
        clock: clock.handle(),
    }
}

fn started_status() -> ROISyncStatus {
    let mut status = blank_status();
    status.running = true;
    status.phase = ROISyncPhase::Spend;
    status.started_at = Some(START.to_owned());
    status
}

#[test]
fn repositories_change_moves_to_the_repository_phase() {
    let clock = TestClock::at("2026-01-10T00:00:00Z");
    let reporter = reporter(started_status(), &clock);
    reporter.apply(StatusChange::Repositories);
    let status = reporter.snapshot();
    assert_eq!(status.phase, ROISyncPhase::Repositories);
    assert_eq!(status.stage, "Reading configured repositories");
    assert!(status.running);
    assert_eq!(status.started_at.as_deref(), Some(START));
}

#[test]
fn estimates_change_records_the_estimate_total() {
    let clock = TestClock::at("2026-01-10T00:00:00Z");
    let reporter = reporter(started_status(), &clock);
    reporter.apply(StatusChange::Estimates { total: 5 });
    let status = reporter.snapshot();
    assert_eq!(status.phase, ROISyncPhase::Estimates);
    assert_eq!(status.stage, "Estimating new or changed pull requests");
    assert_eq!(status.total, 5);
    assert_eq!(status.done, 0);
}

#[test]
fn failed_change_records_the_message() {
    let clock = TestClock::at("2026-01-10T00:00:00Z");
    let reporter = reporter(started_status(), &clock);
    reporter.apply(StatusChange::Failed {
        message: "the gateway refused".to_owned(),
    });
    let status = reporter.snapshot();
    assert_eq!(status.phase, ROISyncPhase::Error);
    assert_eq!(status.stage, "Sync failed");
    assert_eq!(status.error.as_deref(), Some("the gateway refused"));
    assert!(status.running);
}

#[test]
fn cancelled_change_stops_the_run() {
    let clock = TestClock::at("2026-01-10T00:00:00Z");
    let reporter = reporter(started_status(), &clock);
    reporter.apply(StatusChange::Cancelled);
    let status = reporter.snapshot();
    assert_eq!(status.phase, ROISyncPhase::Cancelled);
    assert_eq!(status.stage, "Sync cancelled");
    assert!(!status.running);
    assert!(status.finished_at.is_none());
}

#[test]
fn finished_change_stamps_the_finish_time() {
    let clock = TestClock::at("2026-01-10T00:01:30Z");
    let reporter = reporter(started_status(), &clock);
    reporter.apply(StatusChange::Finished);
    let status = reporter.snapshot();
    assert!(!status.running);
    assert_eq!(status.phase, ROISyncPhase::Spend);
    assert_eq!(
        status.finished_at.as_deref(),
        Some("2026-01-10T00:01:30+00:00")
    );
    assert_eq!(status.elapsed_seconds, 90);
}

#[test]
fn the_computed_finish_materializes_the_timers() {
    let clock = TestClock::at("2026-01-10T00:00:03Z");
    let mut stored = started_status();
    stored.done = 4;
    stored.total = 8;
    let reporter = reporter(stored, &clock);
    reporter.apply(StatusChange::FinishedComputed);
    let status = reporter.snapshot();
    assert_eq!(
        status.finished_at.as_deref(),
        Some("2026-01-10T00:00:03+00:00")
    );
    assert_eq!(status.elapsed_seconds, 3);
    assert!(status.remaining_seconds.is_none());
}

#[test]
fn changes_keep_the_fields_they_do_not_set() {
    let clock = TestClock::at("2026-01-10T00:00:00Z");
    let mut stored = started_status();
    stored.error = Some("kept".to_owned());
    stored.done = 2;
    let reporter = reporter(stored, &clock);
    reporter.apply(StatusChange::Estimates { total: 4 });
    let status = reporter.snapshot();
    assert_eq!(status.error.as_deref(), Some("kept"));
    assert_eq!(status.done, 2);
    assert_eq!(status.total, 4);
    assert_eq!(status.started_at.as_deref(), Some(START));
}
