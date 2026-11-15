#![cfg(test)]

use std::time::SystemTime;

use chrono::{DateTime, Utc};

use super::support::{at, dates, window};
use super::window::spend_logs_window;

fn text(moment: SystemTime) -> String {
    DateTime::<Utc>::from(moment)
        .format("%Y-%m-%dT%H:%M:%S%.6f")
        .to_string()
}

#[test]
fn spend_window_expands_one_before_and_two_after_a_single_date() {
    let computed = spend_logs_window(&dates(&["2026-01-05"])).unwrap();
    assert_eq!(computed, window(at(2026, 1, 4, 0, 0), at(2026, 1, 7, 0, 0)));
}

#[test]
fn spend_window_uses_the_earliest_and_latest_dates() {
    let computed = spend_logs_window(&dates(&["2026-01-09", "2026-01-05", "2026-01-07"])).unwrap();
    assert_eq!(
        computed,
        window(at(2026, 1, 4, 0, 0), at(2026, 1, 11, 0, 0))
    );
}

#[test]
fn spend_window_is_none_for_an_empty_set() {
    assert!(spend_logs_window(&dates(&[])).is_none());
}

#[test]
fn spend_window_is_none_when_no_date_parses() {
    assert!(spend_logs_window(&dates(&["not-a-date", "2026-02-30"])).is_none());
}

#[test]
fn spend_window_ignores_dates_that_do_not_parse() {
    let computed = spend_logs_window(&dates(&["2026-01-05", "not-a-date"])).unwrap();
    assert_eq!(computed, window(at(2026, 1, 4, 0, 0), at(2026, 1, 7, 0, 0)));
}

#[test]
fn spend_window_keeps_the_time_of_day() {
    let computed = spend_logs_window(&dates(&["2026-01-05T10:30:00"])).unwrap();
    assert_eq!(
        computed,
        window(at(2026, 1, 4, 10, 30), at(2026, 1, 7, 10, 30))
    );
}

#[test]
fn spend_window_parses_a_space_separated_datetime() {
    let computed = spend_logs_window(&dates(&["2026-01-05 10:30:00"])).unwrap();
    assert_eq!(
        computed,
        window(at(2026, 1, 4, 10, 30), at(2026, 1, 7, 10, 30))
    );
}

#[test]
fn spend_window_normalizes_an_offset_to_utc() {
    let computed = spend_logs_window(&dates(&["2026-01-05T10:30:00+02:00"])).unwrap();
    assert_eq!(
        computed,
        window(at(2026, 1, 4, 8, 30), at(2026, 1, 7, 8, 30))
    );
}

#[test]
fn spend_window_normalizes_a_mixed_naive_and_offset_set_to_utc() {
    let computed = spend_logs_window(&dates(&["2026-01-05", "2026-01-05T10:30:00+02:00"])).unwrap();
    assert_eq!(
        computed,
        window(at(2026, 1, 4, 0, 0), at(2026, 1, 7, 8, 30))
    );
}

#[test]
fn spend_window_accepts_a_z_suffixed_datetime() {
    let computed = spend_logs_window(&dates(&["2026-01-05T10:30:00Z"])).unwrap();
    assert_eq!(
        computed,
        window(at(2026, 1, 4, 10, 30), at(2026, 1, 7, 10, 30))
    );
}

#[test]
fn spend_window_treats_z_as_an_offset_marker() {
    assert!(spend_logs_window(&dates(&["2026-01-05Z10:30:00"])).is_none());
}

#[test]
fn spend_window_keeps_fractional_seconds() {
    let computed = spend_logs_window(&dates(&["2026-01-05T10:30:00.123456"])).unwrap();
    assert_eq!(text(computed.0), "2026-01-04T10:30:00.123456");
    assert_eq!(text(computed.1), "2026-01-07T10:30:00.123456");
}

#[test]
fn spend_window_is_none_when_the_expansion_leaves_the_calendar() {
    assert!(spend_logs_window(&dates(&["0001-01-01"])).is_none());
    assert!(spend_logs_window(&dates(&["9999-12-31"])).is_none());
}
