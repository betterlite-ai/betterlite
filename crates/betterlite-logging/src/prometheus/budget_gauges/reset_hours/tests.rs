#![cfg(test)]

use chrono::DateTime;

use super::remaining_hours_for_budget_reset;

fn at(seconds: i64) -> DateTime<chrono::Utc> {
    DateTime::from_timestamp(seconds, 0).unwrap()
}

#[test]
fn two_hours_ahead_is_two_hours() {
    assert_eq!(remaining_hours_for_budget_reset(at(7200), at(0)), 2.0);
}

#[test]
fn a_passed_reset_time_is_negative() {
    assert_eq!(remaining_hours_for_budget_reset(at(0), at(3600)), -1.0);
}

#[test]
fn half_an_hour_stays_half() {
    assert_eq!(remaining_hours_for_budget_reset(at(1800), at(0)), 0.5);
}

#[test]
fn the_subsecond_part_counts() {
    let reset = DateTime::from_timestamp(3600, 500_000_000).unwrap();
    assert_eq!(
        remaining_hours_for_budget_reset(reset, at(0)),
        3600.5 / 3600.0
    );
}

#[test]
fn the_two_clocks_share_one_instant() {
    assert_eq!(remaining_hours_for_budget_reset(at(0), at(0)), 0.0);
}
