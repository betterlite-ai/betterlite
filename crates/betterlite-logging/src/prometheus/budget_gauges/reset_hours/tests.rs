#![cfg(test)]

use std::ops::Div as _;

use chrono::DateTime;

use super::hours_until_reset;

fn at(seconds: i64) -> DateTime<chrono::Utc> {
    DateTime::from_timestamp(seconds, 0).unwrap()
}

#[test]
fn two_hours_ahead_is_two_hours() {
    assert_eq!(hours_until_reset(at(7200), at(0)), 2.0);
}

#[test]
fn a_passed_reset_time_is_negative() {
    assert_eq!(hours_until_reset(at(0), at(3600)), -1.0);
}

#[test]
fn half_an_hour_stays_half() {
    assert_eq!(hours_until_reset(at(1800), at(0)), 0.5);
}

#[test]
fn the_subsecond_part_counts() {
    let reset = DateTime::from_timestamp(3600, 500_000_000).unwrap();
    assert_eq!(hours_until_reset(reset, at(0)), 3600.5_f64.div(3600.0_f64));
}

#[test]
fn the_two_clocks_share_one_instant() {
    assert_eq!(hours_until_reset(at(0), at(0)), 0.0);
}
