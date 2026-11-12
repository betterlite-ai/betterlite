#![cfg(test)]

use std::ops::Sub as _;

use jiff::Timestamp;
use rstest::rstest;

use super::{
    RetryAttempt, RetryTiming, calculate_retry_after, retry_after_from_header, should_retry,
};

// 21 Oct 2015 07:28:00 GMT, the example date of the HTTP specification.
const EXAMPLE_DATE: i64 = 1_445_412_480;
const EXAMPLE_HEADER: &str = "Wed, 21 Oct 2015 07:28:00 GMT";
const NOW: i64 = 1_445_412_450;
const ONE_SECOND_BEFORE: i64 = 1_445_412_479;

fn at(seconds: i64) -> Timestamp {
    Timestamp::from_second(seconds).expect("the second is in range")
}

fn close(actual: f64, expected: f64) -> bool {
    actual.sub(expected).abs() < 1e-9_f64
}

fn attempt(remaining_retries: i32, max_retries: i32, min_timeout: i32) -> RetryAttempt {
    RetryAttempt {
        remaining_retries,
        max_retries,
        min_timeout,
    }
}

fn wait(header: Option<&str>, attempt_state: &RetryAttempt, random_unit: f64) -> f64 {
    calculate_retry_after(
        &RetryTiming::default(),
        attempt_state,
        header,
        at(NOW),
        random_unit,
    )
}

#[rstest]
#[case::zero(0, false)]
#[case::continue_status(100, false)]
#[case::ok(200, false)]
#[case::redirect(301, false)]
#[case::bad_request(400, false)]
#[case::unauthorized(401, false)]
#[case::forbidden(403, false)]
#[case::not_found(404, false)]
#[case::just_below_timeout(407, false)]
#[case::request_timeout(408, true)]
#[case::conflict(409, true)]
#[case::gone(410, false)]
#[case::just_below_rate_limit(428, false)]
#[case::rate_limit(429, true)]
#[case::just_above_rate_limit(430, false)]
#[case::last_client_error(499, false)]
#[case::first_server_error(500, true)]
#[case::bad_gateway(502, true)]
#[case::last_defined_server_error(599, true)]
#[case::beyond_defined_codes(999, true)]
fn retry_after_should_retry_only_on_408_409_429_and_500_up(
    #[case] status: u16,
    #[case] expected: bool,
) {
    assert_eq!(should_retry(status), expected);
}

#[rstest]
#[case::whole_seconds(Some("30"), 30)]
#[case::with_spaces(Some("  30  "), 30)]
#[case::with_newline(Some("30\n"), 30)]
#[case::zero(Some("0"), 0)]
#[case::negative(Some("-5"), -5)]
#[case::plus_sign(Some("+7"), 7)]
#[case::above_the_wait_limit(Some("3600"), 3600)]
#[case::missing(None, -1)]
#[case::empty(Some(""), -1)]
#[case::blank(Some("   "), -1)]
#[case::words(Some("soon"), -1)]
#[case::fraction(Some("3.5"), -1)]
#[case::date_thirty_seconds_ahead(Some(EXAMPLE_HEADER), 30)]
#[case::date_with_offset(Some("Wed, 21 Oct 2015 09:28:00 +0200"), 30)]
#[case::date_without_weekday(Some("21 Oct 2015 07:28:00 GMT"), 30)]
#[case::date_in_the_past(Some("Wed, 21 Oct 2015 07:26:00 GMT"), -90)]
#[case::date_with_a_month_that_does_not_exist(Some("Wed, 21 Octo 2015 07:28:00 GMT"), -1)]
fn retry_after_header_reads_seconds_or_an_http_date(
    #[case] header: Option<&str>,
    #[case] expected: i64,
) {
    assert_eq!(retry_after_from_header(header, at(NOW)), expected);
}

#[test]
fn retry_after_header_date_counts_from_the_given_now() {
    assert_eq!(
        retry_after_from_header(Some(EXAMPLE_HEADER), at(EXAMPLE_DATE)),
        0
    );
    assert_eq!(
        retry_after_from_header(Some(EXAMPLE_HEADER), at(ONE_SECOND_BEFORE)),
        1
    );
}

#[rstest]
#[case::one_second(Some("1"), 1.0_f64)]
#[case::mid_range(Some("30"), 30.0_f64)]
#[case::last_second_of_the_window(Some("60"), 60.0_f64)]
#[case::http_date_inside_the_window(Some(EXAMPLE_HEADER), 30.0_f64)]
fn retry_after_calculate_a_header_inside_the_window_wins_without_jitter_at_zero(
    #[case] header: Option<&str>,
    #[case] expected: f64,
) {
    assert!(close(wait(header, &attempt(0, 3, 0), 0.0), expected));
}

#[rstest]
#[case::zero(Some("0"))]
#[case::negative(Some("-1"))]
#[case::just_above_the_window(Some("61"))]
#[case::far_above_the_window(Some("3600"))]
#[case::beyond_i64(Some("99999999999999999999"))]
#[case::words(Some("soon"))]
#[case::missing(None)]
#[case::past_date(Some("Wed, 21 Oct 2015 07:26:00 GMT"))]
fn retry_after_calculate_a_header_outside_the_window_falls_to_backoff(
    #[case] header: Option<&str>,
) {
    assert!(close(wait(header, &attempt(3, 3, 0), 0.0), 0.5));
}

#[rstest]
#[case::no_jitter(0.0_f64, 31.0_f64)]
#[case::quarter(0.25_f64, 31.1875_f64)]
#[case::half(0.5_f64, 31.375_f64)]
#[case::full(1.0_f64, 31.75_f64)]
fn retry_after_calculate_jitter_is_added_to_a_header_wait(
    #[case] random_unit: f64,
    #[case] expected: f64,
) {
    assert!(close(
        wait(Some("31"), &attempt(3, 3, 0), random_unit),
        expected
    ));
}

#[rstest]
#[case::first_try(3_i32, 3_i32, 0_i32, 0.5_f64)]
#[case::second_try(2_i32, 3_i32, 0_i32, 1.0_f64)]
#[case::third_try(1_i32, 3_i32, 0_i32, 2.0_f64)]
#[case::fourth_try(0_i32, 3_i32, 0_i32, 4.0_f64)]
#[case::reaches_the_cap(0_i32, 4_i32, 0_i32, 8.0_f64)]
#[case::above_the_cap(0_i32, 5_i32, 0_i32, 8.0_f64)]
#[case::far_above_the_cap(0_i32, 30_i32, 0_i32, 8.0_f64)]
#[case::remaining_above_max(5_i32, 3_i32, 0_i32, 0.125_f64)]
#[case::min_timeout_raises_a_short_wait(3_i32, 3_i32, 5_i32, 5.0_f64)]
#[case::min_timeout_below_the_wait_changes_nothing(0_i32, 3_i32, 2_i32, 4.0_f64)]
#[case::min_timeout_equal_to_the_wait(0_i32, 3_i32, 4_i32, 4.0_f64)]
#[case::cap_wins_over_min_timeout(3_i32, 3_i32, 10_i32, 8.0_f64)]
#[case::negative_min_timeout(3_i32, 3_i32, -4_i32, 0.5_f64)]
fn retry_after_calculate_backoff_doubles_then_boxes_between_min_timeout_and_cap(
    #[case] remaining: i32,
    #[case] max: i32,
    #[case] min_timeout: i32,
    #[case] expected: f64,
) {
    assert!(close(
        wait(None, &attempt(remaining, max, min_timeout), 0.0),
        expected
    ));
}

#[test]
fn retry_after_calculate_backoff_adds_jitter_after_the_cap() {
    let state = attempt(0, 10, 0);
    assert!(close(wait(None, &state, 0.0), 8.0));
    assert!(close(wait(None, &state, 1.0), 8.75));
}

#[test]
fn retry_after_calculate_backoff_uses_the_passed_constants() {
    let timing = RetryTiming {
        initial_retry_delay: 1.0,
        max_retry_delay: 100.0,
        jitter: 2.0,
    };
    let state = attempt(1, 3, 0);
    let now = at(EXAMPLE_DATE);
    assert!(close(
        calculate_retry_after(&timing, &state, None, now, 0.0),
        4.0
    ));
    assert!(close(
        calculate_retry_after(&timing, &state, None, now, 0.5),
        5.0
    ));
    assert!(close(
        calculate_retry_after(&timing, &attempt(0, 10, 0), None, now, 0.0),
        100.0
    ));
}

#[test]
fn retry_after_calculate_a_header_wait_ignores_the_backoff_constants() {
    let timing = RetryTiming {
        initial_retry_delay: 9.0,
        max_retry_delay: 1.0,
        jitter: 0.0,
    };
    let seconds = calculate_retry_after(&timing, &attempt(3, 3, 7), Some("20"), at(0), 0.9);
    assert!(close(seconds, 20.0));
}

#[test]
fn retry_after_calculate_defaults_are_the_litellm_constants() {
    let timing = RetryTiming::default();
    assert!(close(timing.initial_retry_delay, 0.5));
    assert!(close(timing.max_retry_delay, 8.0));
    assert!(close(timing.jitter, 0.75));
}

#[test]
fn retry_after_calculate_a_retry_count_past_the_float_range_caps_at_the_maximum() {
    let seconds = wait(None, &attempt(0, 2000, 0), 0.0);
    assert!(close(seconds, 8.0));
}
