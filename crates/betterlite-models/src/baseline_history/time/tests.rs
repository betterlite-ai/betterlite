#![cfg(test)]

use rstest::rstest;

use super::{checked_add_timestamp, ttl_seconds_as_f64};
use crate::baseline_history::error::BaselineHistoryError;

#[rstest]
#[case(300_i64, Ok(300.0_f64))]
#[case(3600_i64, Ok(3600.0_f64))]
#[case(60_i64, Err(BaselineHistoryError::InvalidTtl))]
#[case(0_i64, Err(BaselineHistoryError::InvalidTtl))]
fn ttl_seconds_as_f64_accepts_only_the_valid_domain(
    #[case] ttl_seconds: i64,
    #[case] expected: Result<f64, BaselineHistoryError>,
) {
    assert_eq!(ttl_seconds_as_f64(ttl_seconds), expected);
}

#[test]
fn checked_add_timestamp_returns_the_finite_sum() {
    assert_eq!(checked_add_timestamp(10.0_f64, 3600.0_f64), Ok(3610.0_f64));
}

#[test]
fn checked_add_timestamp_rejects_an_infinite_result() {
    let error = checked_add_timestamp(f64::MAX, f64::MAX).expect_err("overflowed to infinity");
    assert_eq!(error, BaselineHistoryError::TimestampOutOfRange);
}
