#![cfg(test)]

use std::ops::Div as _;

use chrono::DateTime;

use super::hours_until_reset;

fn at(seconds: i64) -> DateTime<chrono::Utc> {
    DateTime::from_timestamp(seconds, 0).unwrap()
}

#[test]
fn two_hours_ahead_is_two_hours() {
    assert!(
        hours_until_reset(at(7200), at(0))
            .total_cmp(&2.0_f64)
            .is_eq()
    );
}

#[test]
fn a_passed_reset_time_is_negative() {
    assert!(
        hours_until_reset(at(0), at(3600))
            .total_cmp(&-1.0_f64)
            .is_eq()
    );
}

#[test]
fn half_an_hour_stays_half() {
    assert!(
        hours_until_reset(at(1800), at(0))
            .total_cmp(&0.5_f64)
            .is_eq()
    );
}

#[test]
fn the_subsecond_part_counts() {
    let reset = DateTime::from_timestamp(3600, 500_000_000).unwrap();
    let expected = 3600.5_f64.div(3600.0_f64);
    assert!(hours_until_reset(reset, at(0)).total_cmp(&expected).is_eq());
}

#[test]
fn the_two_clocks_share_one_instant() {
    assert!(hours_until_reset(at(0), at(0)).total_cmp(&0.0_f64).is_eq());
}
