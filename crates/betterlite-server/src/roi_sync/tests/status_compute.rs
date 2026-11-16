#![cfg(test)]

use std::sync::{Arc, Mutex};

use betterlite_roi::records::{ROISyncPhase, ROISyncStatus};
use tokio::sync::Notify;

use super::support::{TestClock, blank_status, inputs, manager, stalled_caller};
use crate::roi_sync::status::StatusReporter;

const START: &str = "2026-01-10T00:00:00+00:00";

fn reporter(stored: ROISyncStatus, clock: &TestClock) -> StatusReporter {
    StatusReporter {
        stored: Arc::new(Mutex::new(stored)),
        clock: clock.handle(),
    }
}

fn running_status(done: i64, total: i64) -> ROISyncStatus {
    let mut status = blank_status();
    status.running = true;
    status.phase = ROISyncPhase::Estimates;
    status.started_at = Some(START.to_owned());
    status.done = done;
    status.total = total;
    status
}

#[test]
fn status_without_a_start_time_reports_no_elapsed_time() {
    let clock = TestClock::at("2026-01-10T00:05:00Z");
    let status = reporter(blank_status(), &clock).snapshot();
    assert_eq!(status.elapsed_seconds, 0);
    assert!(status.remaining_seconds.is_none());
}

#[test]
fn status_counts_elapsed_to_the_finish_time() {
    let clock = TestClock::at("2026-01-10T05:00:00Z");
    let mut stored = running_status(2, 6);
    stored.running = false;
    stored.finished_at = Some("2026-01-10T00:01:30+00:00".to_owned());
    assert_eq!(reporter(stored, &clock).snapshot().elapsed_seconds, 90);
}

#[test]
fn status_counts_elapsed_to_now_while_the_run_is_running() {
    let clock = TestClock::at("2026-01-10T00:00:45Z");
    let status = reporter(running_status(1, 6), &clock).snapshot();
    assert_eq!(status.elapsed_seconds, 45);
    assert!(status.remaining_seconds.is_none());
}

#[test]
fn status_estimates_remaining_time_after_three_done() {
    let clock = TestClock::at("2026-01-10T00:00:30Z");
    let status = reporter(running_status(3, 6), &clock).snapshot();
    assert_eq!(status.elapsed_seconds, 30);
    assert_eq!(status.remaining_seconds, Some(30));
}

#[test]
fn status_rounds_a_half_towards_the_even_second() {
    let early = TestClock::at("2026-01-10T00:00:01Z");
    let even = reporter(running_status(4, 6), &early).snapshot();
    assert_eq!(even.remaining_seconds, Some(0));
    let late = TestClock::at("2026-01-10T00:00:03Z");
    let odd = reporter(running_status(4, 6), &late).snapshot();
    assert_eq!(odd.remaining_seconds, Some(2));
}

#[test]
fn status_rounds_a_fraction_of_a_second_up() {
    let clock = TestClock::at("2026-01-10T00:00:04Z");
    let status = reporter(running_status(3, 8), &clock).snapshot();
    assert_eq!(status.remaining_seconds, Some(7));
}

#[test]
fn status_reports_no_remaining_time_for_a_finished_run() {
    let clock = TestClock::at("2026-01-10T05:00:00Z");
    let mut stored = running_status(3, 6);
    stored.running = false;
    stored.finished_at = Some("2026-01-10T00:00:10+00:00".to_owned());
    let status = reporter(stored, &clock).snapshot();
    assert_eq!(status.elapsed_seconds, 10);
    assert!(status.remaining_seconds.is_none());
}

#[test]
fn status_reports_no_remaining_time_below_three_done() {
    let clock = TestClock::at("2026-01-10T00:00:30Z");
    let status = reporter(running_status(2, 6), &clock).snapshot();
    assert!(status.remaining_seconds.is_none());
}

#[test]
fn status_floors_a_remaining_time_at_zero() {
    let clock = TestClock::at("2026-01-10T00:00:03Z");
    let status = reporter(running_status(5, 3), &clock).snapshot();
    assert_eq!(status.remaining_seconds, Some(0));
}

#[test]
fn status_ignores_unparseable_timestamps() {
    let clock = TestClock::at("2026-01-10T00:00:30Z");
    let mut stored = running_status(4, 6);
    stored.started_at = Some("yesterday".to_owned());
    let status = reporter(stored, &clock).snapshot();
    assert_eq!(status.elapsed_seconds, 0);
    assert_eq!(status.remaining_seconds, Some(0));
}

#[test]
fn status_clamps_a_finish_before_the_start() {
    let clock = TestClock::at("2026-01-10T00:00:00Z");
    let mut stored = running_status(2, 6);
    stored.running = false;
    stored.finished_at = Some("2026-01-09T23:00:00+00:00".to_owned());
    assert_eq!(reporter(stored, &clock).snapshot().elapsed_seconds, 0);
}

#[tokio::test]
async fn status_from_the_manager_reads_the_injected_clock() {
    let clock = TestClock::at("2026-04-01T00:00:00Z");
    let manager = manager(Some(clock.handle()));
    let mut inputs = inputs("o/r", &[12]);
    inputs.complete = stalled_caller(Arc::new(Notify::new()));
    assert!(manager.start(inputs.run()).await);
    clock.set("2026-04-01T00:00:07Z");
    let status = manager.status();
    assert_eq!(
        status.started_at.as_deref(),
        Some("2026-04-01T00:00:00+00:00")
    );
    assert_eq!(status.elapsed_seconds, 7);
    assert!(status.remaining_seconds.is_none());
    assert!(manager.cancel().await);
}
