#![cfg(test)]

use chrono::{DateTime, Utc};
use http::StatusCode;
use rstest::rstest;

use super::{UsageWindow, UsageWindowError, resolve_usage_window};
use crate::guardrails::usage_projections::USAGE_MAX_RANGE_DAYS;

fn clock(text: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(text)
        .unwrap()
        .with_timezone(&Utc)
}

fn day(start: &str, end: &str) -> UsageWindow {
    UsageWindow {
        start: start.to_owned(),
        end: end.to_owned(),
    }
}

#[rstest]
#[case::one_digit_month("2026-1-1")]
#[case::one_digit_day("2026-01-1")]
#[case::day_out_of_range("2026-11-31")]
#[case::american_order("16-11-2026")]
#[case::compact("20261116")]
#[case::timestamp("2026-11-16T00:00:00")]
#[case::signed_year("+2026-01-01")]
#[case::year_zero("0000-01-01")]
#[case::five_digit_year("10000-01-01")]
#[case::leading_space(" 2026-01-01")]
fn window_validation_reports_bad_format(#[case] bad: &str) {
    let now = clock("2026-11-16T00:00:00Z");
    assert_eq!(
        resolve_usage_window(Some(bad), None, now),
        Err(UsageWindowError::DateFormat)
    );
    assert_eq!(
        resolve_usage_window(None, Some(bad), now),
        Err(UsageWindowError::DateFormat)
    );
}

#[test]
fn window_validation_names_the_format_error() {
    assert_eq!(
        UsageWindowError::DateFormat.to_string(),
        "start_date and end_date must be in YYYY-MM-DD format"
    );
}

#[test]
fn window_validation_reports_out_of_order_dates() {
    let now = clock("2026-11-16T00:00:00Z");
    assert_eq!(
        resolve_usage_window(Some("2026-11-05"), Some("2026-11-01"), now),
        Err(UsageWindowError::OutOfOrder)
    );
    assert_eq!(
        UsageWindowError::OutOfOrder.to_string(),
        "start_date must be on or before end_date"
    );
    assert_eq!(
        resolve_usage_window(Some("2026-11-01"), Some("2026-11-01"), now),
        Ok(day("2026-11-01", "2026-11-01"))
    );
}

#[test]
fn window_validation_caps_the_range_at_366_days() {
    let now = clock("2026-11-16T00:00:00Z");
    assert_eq!(
        resolve_usage_window(Some("2026-01-01"), Some("2027-01-02"), now),
        Ok(day("2026-01-01", "2027-01-02"))
    );
    assert_eq!(
        resolve_usage_window(Some("2026-01-01"), Some("2027-01-03"), now),
        Err(UsageWindowError::RangeTooLarge)
    );
    assert_eq!(
        UsageWindowError::RangeTooLarge.to_string(),
        format!("Date range too large; maximum is {USAGE_MAX_RANGE_DAYS} days")
    );
}

#[test]
fn window_validation_counts_calendar_days_across_leap_years() {
    let now = clock("2028-03-01T00:00:00Z");
    assert_eq!(
        resolve_usage_window(Some("2028-01-01"), Some("2029-01-01"), now),
        Ok(day("2028-01-01", "2029-01-01"))
    );
    assert_eq!(
        resolve_usage_window(Some("2028-01-01"), Some("2029-01-02"), now),
        Err(UsageWindowError::RangeTooLarge)
    );
}

#[test]
fn window_defaults_run_from_seven_days_back_to_utc_today() {
    let late = clock("2026-11-16T23:59:59Z");
    assert_eq!(
        resolve_usage_window(None, None, late),
        Ok(day("2026-11-09", "2026-11-16"))
    );
    assert_eq!(
        resolve_usage_window(Some("2026-10-01"), None, late),
        Ok(day("2026-10-01", "2026-11-16"))
    );
}

#[test]
fn window_defaults_keep_an_explicit_end_and_still_start_seven_days_back() {
    let now = clock("2026-11-16T00:00:00Z");
    assert_eq!(
        resolve_usage_window(None, Some("2026-11-10"), now),
        Ok(day("2026-11-09", "2026-11-10"))
    );
    assert_eq!(
        resolve_usage_window(None, Some("2026-11-01"), now),
        Err(UsageWindowError::OutOfOrder)
    );
}

#[test]
fn format_strict_accepts_four_digit_years_only() {
    let now = clock("2026-11-16T00:00:00Z");
    assert_eq!(
        resolve_usage_window(Some("1000-01-01"), Some("1000-01-02"), now),
        Ok(day("1000-01-01", "1000-01-02"))
    );
    assert_eq!(
        resolve_usage_window(Some("9999-12-31"), Some("9999-12-31"), now),
        Ok(day("9999-12-31", "9999-12-31"))
    );
}

#[test]
fn format_strict_keeps_the_normalized_window_strings() {
    let now = clock("2026-11-16T00:00:00Z");
    let resolved = resolve_usage_window(Some("2026-11-01"), Some("2026-11-16"), now).unwrap();
    assert_eq!(resolved.start, "2026-11-01");
    assert_eq!(resolved.end, "2026-11-16");
}

#[rstest]
#[case::format(UsageWindowError::DateFormat)]
#[case::order(UsageWindowError::OutOfOrder)]
#[case::range(UsageWindowError::RangeTooLarge)]
fn window_validation_answers_every_error_with_a_400_bad_request(#[case] error: UsageWindowError) {
    assert_eq!(error.status_code(), StatusCode::BAD_REQUEST);
    assert_eq!(error.error_type(), "bad_request");
}
