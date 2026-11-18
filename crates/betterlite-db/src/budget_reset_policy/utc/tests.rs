#![cfg(test)]

use super::count_advanced;
use crate::budget_permission_rows::Timestamp;
use chrono::{DateTime, NaiveDate, TimeZone as _, Utc};

fn cutoff(hour: u32, minute: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 1, 1, hour, minute, 0).unwrap()
}

fn aware(text: &str) -> Option<Timestamp> {
    DateTime::parse_from_rfc3339(text)
        .ok()
        .map(Timestamp::Aware)
}

fn naive(hour: u32, minute: u32) -> Option<Timestamp> {
    NaiveDate::from_ymd_opt(2026, 1, 1)
        .and_then(|date| date.and_hms_opt(hour, minute, 0))
        .map(Timestamp::Naive)
}

#[test]
fn utc_aware_timestamp_after_the_cutoff_advances() {
    assert_eq!(
        count_advanced([aware("2026-01-01T12:00:00+02:00")], cutoff(9, 0)),
        1
    );
}

#[test]
fn utc_aware_timestamp_is_compared_at_its_own_instant() {
    assert_eq!(
        count_advanced([aware("2026-01-01T12:00:00+02:00")], cutoff(10, 30)),
        0
    );
}

#[test]
fn utc_naive_timestamp_is_treated_as_utc() {
    assert_eq!(count_advanced([naive(10, 0)], cutoff(9, 0)), 1);
}

#[test]
fn utc_naive_timestamp_behind_the_cutoff_does_not_advance() {
    assert_eq!(count_advanced([naive(9, 0)], cutoff(10, 0)), 0);
}

#[test]
fn utc_timestamp_equal_to_the_cutoff_does_not_advance() {
    assert_eq!(
        count_advanced([aware("2026-01-01T10:00:00Z")], cutoff(10, 0)),
        0
    );
}

#[test]
fn utc_missing_timestamp_does_not_advance() {
    assert_eq!(count_advanced([None], cutoff(10, 0)), 0);
}

#[test]
fn utc_counts_only_the_rows_past_the_cutoff() {
    assert_eq!(
        count_advanced(
            [
                naive(11, 0),
                None,
                aware("2026-01-01T05:00:00-05:00"),
                aware("2026-01-01T10:00:01Z"),
            ],
            cutoff(10, 0),
        ),
        2
    );
}
