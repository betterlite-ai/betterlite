#![cfg(test)]

use chrono::{DateTime, NaiveDate, Utc};

use crate::reload_schedule::ReloadSchedule;
use crate::reload_schedule::next_run_at::next_run_at;

fn at(day: u32, hour: u32) -> DateTime<Utc> {
    NaiveDate::from_ymd_opt(2026, 3, day)
        .unwrap()
        .and_hms_opt(hour, 6, 7)
        .unwrap()
        .and_utc()
}

fn schedule(interval_hours: Option<u32>, last_run_at: Option<DateTime<Utc>>) -> ReloadSchedule {
    ReloadSchedule {
        interval_hours,
        reload_revision: 1,
        last_run_at,
    }
}

#[test]
fn next_run_at_needs_the_interval() {
    assert_eq!(next_run_at(&schedule(None, Some(at(4, 5)))), None);
}

#[test]
fn next_run_at_needs_the_last_run() {
    assert_eq!(next_run_at(&schedule(Some(6), None)), None);
}

#[test]
fn next_run_at_adds_the_interval_to_the_last_run() {
    assert_eq!(
        next_run_at(&schedule(Some(6), Some(at(4, 5)))),
        Some(at(4, 11))
    );
}

#[test]
fn next_run_at_crosses_the_day_boundary() {
    assert_eq!(
        next_run_at(&schedule(Some(6), Some(at(4, 20)))),
        Some(at(5, 2))
    );
}

#[test]
fn next_run_at_keeps_the_last_run_for_a_zero_interval() {
    assert_eq!(
        next_run_at(&schedule(Some(0), Some(at(4, 5)))),
        Some(at(4, 5))
    );
}

#[test]
fn next_run_at_reports_no_run_when_the_sum_leaves_the_calendar() {
    assert_eq!(next_run_at(&schedule(Some(u32::MAX), Some(at(4, 5)))), None);
}
