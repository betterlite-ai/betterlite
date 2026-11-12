use std::collections::BTreeMap;

use betterlite_config::sections::retry_policy::RetryPolicySettings;
use rstest::rstest;

use super::{FixedSample, availability, every_field, seconds, settings};
use crate::cooldown_decisions::failure::FailureClass;
use crate::retry_policy::backoff::{RetryTiming, sleep_before_retry};
use crate::retry_policy::eligibility::{
    Availability, RetryDecision, StopReason, decide, is_retryable_status,
};
use crate::retry_policy::policy::{RetryCounts, RetryPolicies, RetryPolicy, plan};
use crate::retry_policy::retry_after::{
    MAX_RETRY_AFTER_BYTES, RetryAfter, RetryAfterError, parse_retry_after,
};
use crate::retry_policy::skip::{FailedDeployment, skipped_deployment_ids};

#[rstest]
#[case::request_timeout(408, true)]
#[case::lock_timeout(409, true)]
#[case::rate_limit(429, true)]
#[case::internal_server(500, true)]
#[case::not_implemented(501, true)]
#[case::ok(200, false)]
#[case::bad_request(400, false)]
#[case::authentication(401, false)]
#[case::permission_denied(403, false)]
#[case::not_found(404, false)]
#[case::unprocessable(422, false)]
#[case::client_closed(499, false)]
fn only_timeouts_rate_limits_and_server_errors_are_retryable(
    #[case] status: u16,
    #[case] retryable: bool,
) {
    assert_eq!(is_retryable_status(status), retryable);
}

#[rstest]
#[case::authentication_status_is_kept(
    FailureClass::Unclassified,
    Some(401),
    availability(2, 2),
    RetryDecision::Retry
)]
#[case::permission_status_is_kept(
    FailureClass::PermissionDenied,
    Some(403),
    availability(2, 2),
    RetryDecision::Retry
)]
#[case::bad_request_stops(
    FailureClass::Unclassified,
    Some(400),
    availability(2, 2),
    RetryDecision::Stop(StopReason::StatusNotRetryable)
)]
#[case::not_found_status_stops_before_the_class(
    FailureClass::NotFound,
    Some(404),
    availability(2, 2),
    RetryDecision::Stop(StopReason::StatusNotRetryable)
)]
#[case::not_found_class_stops(
    FailureClass::NotFound,
    Some(500),
    availability(2, 2),
    RetryDecision::Stop(StopReason::NotFound)
)]
#[case::missing_status_skips_the_check(
    FailureClass::Unclassified,
    None,
    availability(2, 2),
    RetryDecision::Retry
)]
#[case::missing_status_still_needs_a_deployment(
    FailureClass::Unclassified,
    None,
    availability(0, 2),
    RetryDecision::Stop(StopReason::NoHealthyDeployments)
)]
#[case::context_window_without_fallbacks_retries(
    FailureClass::ContextWindowExceeded,
    Some(500),
    availability(2, 2),
    RetryDecision::Retry
)]
#[case::auth_without_any_deployment(
    FailureClass::Authentication,
    Some(401),
    availability(0, 0),
    RetryDecision::Stop(StopReason::AuthWithoutOtherDeployment)
)]
#[case::auth_with_one_deployment_and_healthy_others(
    FailureClass::Authentication,
    Some(401),
    availability(2, 1),
    RetryDecision::Stop(StopReason::AuthWithoutOtherDeployment)
)]
#[case::auth_with_two_deployments_but_none_healthy(
    FailureClass::Authentication,
    Some(401),
    availability(0, 2),
    RetryDecision::Stop(StopReason::NoHealthyDeployments)
)]
#[case::no_deployments_at_all(
    FailureClass::Timeout,
    Some(408),
    availability(0, 0),
    RetryDecision::Stop(StopReason::NoHealthyDeployments)
)]
#[case::healthy_deployment_without_all_deployments(
    FailureClass::Timeout,
    Some(408),
    availability(1, 0),
    RetryDecision::Retry
)]
fn the_exact_stop_reason_follows_the_reference_order(
    #[case] class: FailureClass,
    #[case] status: Option<u16>,
    #[case] availability: Availability,
    #[case] expected: RetryDecision,
) {
    assert_eq!(decide(class, status, &availability), expected);
}

#[rstest]
#[case::authentication(FailureClass::Authentication, Some(500), None)]
#[case::authentication_404(FailureClass::Authentication, Some(404), None)]
#[case::bad_request(FailureClass::BadRequest, Some(500), None)]
#[case::not_found(FailureClass::NotFound, Some(404), None)]
#[case::rate_limit(FailureClass::RateLimit, Some(500), None)]
#[case::unclassified(FailureClass::Unclassified, Some(500), None)]
fn an_empty_policy_grants_nothing(
    #[case] class: FailureClass,
    #[case] status: Option<u16>,
    #[case] expected: Option<i64>,
) {
    assert_eq!(RetryPolicy::default().retries_for(class, status), expected);
}

#[rstest]
#[case::explicit_zero_is_used(Some(0), Some(0))]
#[case::negative_is_used(Some(-1), Some(-1))]
#[case::positive_is_used(Some(8), Some(8))]
fn a_policy_value_is_used_as_given(#[case] not_found: Option<i64>, #[case] expected: Option<i64>) {
    let policy = RetryPolicy {
        not_found,
        ..every_field()
    };
    assert_eq!(
        policy.retries_for(FailureClass::NotFound, Some(404)),
        expected
    );
}

#[test]
fn the_plan_keeps_a_deployment_count_at_the_unsigned_limit() {
    let counts = RetryCounts {
        request: None,
        router: 0,
        deployment: Some(u32::MAX),
    };
    let outcome = plan(
        counts,
        &RetryPolicies::default(),
        FailureClass::RateLimit,
        Some(429),
        None,
    );
    assert_eq!(outcome.num_retries, 0xFFFF_FFFF_i64);
    assert!(!outcome.policy_applies);
}

#[test]
fn the_plan_never_lets_a_policy_override_an_explicit_request_zero() {
    let policies = RetryPolicies {
        global: Some(every_field()),
        model_group: BTreeMap::new(),
    };
    let counts = RetryCounts {
        request: Some(0),
        router: 5,
        deployment: Some(2),
    };
    let outcome = plan(counts, &policies, FailureClass::RateLimit, Some(429), None);
    assert_eq!(outcome.num_retries, 0);
    assert!(!outcome.policy_applies);
}

#[test]
fn a_policy_zero_is_applied_and_stops_the_retries() {
    let policy = RetryPolicy {
        bad_request: None,
        authentication: None,
        timeout: None,
        rate_limit: Some(0),
        content_policy: None,
        internal_server: None,
        service_unavailable: None,
        not_found: None,
        fallback: None,
    };
    let policies = RetryPolicies {
        global: Some(policy),
        model_group: BTreeMap::new(),
    };
    let counts = RetryCounts {
        request: None,
        router: 5,
        deployment: None,
    };
    let outcome = plan(counts, &policies, FailureClass::RateLimit, Some(429), None);
    assert_eq!(outcome.num_retries, 0);
    assert!(outcome.policy_applies);
}

#[test]
fn the_saturation_of_an_unbounded_seconds_value_never_changes_a_decision() {
    let huge = format!("9{}", "9".repeat(40));
    assert_eq!(
        parse_retry_after(huge.as_bytes()).unwrap(),
        RetryAfter::Seconds(i64::MAX)
    );
    let huge_negative = format!("-9{}", "9".repeat(40));
    assert_eq!(
        parse_retry_after(huge_negative.as_bytes()).unwrap(),
        RetryAfter::Seconds(i64::MIN)
    );
    assert_eq!(
        parse_retry_after(i64::MAX.to_string().as_bytes()).unwrap(),
        RetryAfter::Seconds(i64::MAX)
    );
    assert_eq!(
        parse_retry_after(i64::MIN.to_string().as_bytes()).unwrap(),
        RetryAfter::Seconds(i64::MIN)
    );
}

#[rstest]
#[case::zero(b"0", 0)]
#[case::one(b"1", 1)]
#[case::sixty(b"60", 60)]
#[case::sixty_one(b"61", 61)]
#[case::negative_one(b"-1", -1)]
#[case::negative_at_the_signed_limit(b"-9223372036854775807", -0x7FFF_FFFF_FFFF_FFFF_i64)]
fn seconds_are_read_exactly(#[case] bytes: &[u8], #[case] expected: i64) {
    assert_eq!(
        parse_retry_after(bytes).unwrap(),
        RetryAfter::Seconds(expected)
    );
}

#[rstest]
#[case::empty(b"")]
#[case::spaces(b"   ")]
#[case::float(b"5.0")]
#[case::hex(b"0x10")]
#[case::bare_plus(b"+")]
#[case::bare_minus(b"-")]
#[case::words(b"soon")]
#[case::underscore(b"1_0")]
#[case::year_past_9999(b"21 Oct 10000 07:28:00 GMT")]
#[case::year_before_one(b"21 Oct -5000 07:28:00 GMT")]
#[case::unknown_month(b"21 Foo 2015 07:28:00 GMT")]
#[case::three_fields(b"21 Oct 2015")]
#[case::hour_only(b"21 Oct 2015 07 GMT")]
#[case::weekday_only(b"Wed,")]
#[case::four_clock_parts(b"21 Oct 2015 07:28:00:00 GMT")]
#[case::four_dotted_parts(b"21 Oct 2015 1.2.3.4 GMT")]
#[case::letter_in_the_clock(b"21 Oct 2015 x:28:00 GMT")]
#[case::two_zones_no_year(b"21 Oct GMT 07:28:00 UTC")]
#[case::lone_comma_year(b"21 Oct , 07:28 2015")]
#[case::leading_day_comma_is_dropped(b"21, Oct 2015, 07:28:00, GMT")]
fn an_unparsable_value_is_a_typed_error(#[case] bytes: &[u8]) {
    assert!(matches!(
        parse_retry_after(bytes),
        Err(RetryAfterError::NotDate)
    ));
}

#[test]
fn invalid_utf8_is_a_typed_error() {
    assert!(matches!(
        parse_retry_after(b"\xff\xfe"),
        Err(RetryAfterError::NotUtf8 { .. })
    ));
}

#[test]
fn the_byte_limit_is_checked_before_decoding() {
    let over = vec![b'9'; MAX_RETRY_AFTER_BYTES.saturating_add(1)];
    assert!(matches!(
        parse_retry_after(&over),
        Err(RetryAfterError::TooLarge { length })
            if length == MAX_RETRY_AFTER_BYTES.saturating_add(1)
    ));
    let invalid = vec![0xFF_u8; MAX_RETRY_AFTER_BYTES.saturating_add(1)];
    assert!(matches!(
        parse_retry_after(&invalid),
        Err(RetryAfterError::TooLarge { .. })
    ));
}

#[test]
fn the_byte_limit_itself_is_accepted() {
    let digits = vec![b'9'; MAX_RETRY_AFTER_BYTES];
    assert_eq!(
        parse_retry_after(&digits).unwrap(),
        RetryAfter::Seconds(i64::MAX)
    );
    let text = vec![b'x'; MAX_RETRY_AFTER_BYTES];
    assert!(matches!(
        parse_retry_after(&text),
        Err(RetryAfterError::NotDate)
    ));
}

#[rstest]
#[case::exact(b"Thu, 01 Jan 1970 00:00:00 GMT", 0, 0)]
#[case::half_a_second_before(b"Thu, 01 Jan 1970 00:00:00 GMT", -500, 0)]
#[case::one_second_before(b"Thu, 01 Jan 1970 00:00:00 GMT", -1000, 1)]
#[case::half_a_second_after(b"Thu, 01 Jan 1970 00:00:00 GMT", 500, 0)]
#[case::one_second_after(b"Thu, 01 Jan 1970 00:00:00 GMT", 1000, -1)]
fn date_seconds_truncate_toward_zero(
    #[case] bytes: &[u8],
    #[case] now_millis: i64,
    #[case] expected: i64,
) {
    let value = parse_retry_after(bytes).unwrap();
    assert_eq!(value.seconds_until(now_millis), expected);
}

#[rstest]
#[case::zero_falls_back(Some(0), "0.500000")]
#[case::one_is_kept(Some(1), "1.000000")]
#[case::sixty_is_kept(Some(60), "60.000000")]
#[case::sixty_one_falls_back(Some(61), "0.500000")]
#[case::negative_falls_back(Some(-1), "0.500000")]
#[case::minimum_falls_back(Some(i64::MIN), "0.500000")]
#[case::maximum_falls_back(Some(i64::MAX), "0.500000")]
fn the_header_is_obeyed_only_between_one_and_sixty_seconds(
    #[case] retry_after: Option<i64>,
    #[case] expected: &str,
) {
    let mut random = FixedSample::new(0);
    let delay = sleep_before_retry(
        &settings(0.0),
        RetryTiming {
            remaining_retries: 2,
            max_retries: 2,
        },
        &availability(0, 2),
        retry_after,
        &mut random,
    );
    assert_eq!(seconds(delay), expected);
}

#[rstest]
#[case::first(2, 2, "0.500000")]
#[case::fourth(0, 4, "8.000000")]
#[case::clamped(0, 5, "8.000000")]
#[case::negative_exponent(3, 1, "0.125000")]
fn the_exponential_uses_the_spent_retries(
    #[case] remaining_retries: i64,
    #[case] max_retries: i64,
    #[case] expected: &str,
) {
    let mut random = FixedSample::new(0);
    let delay = sleep_before_retry(
        &settings(0.0),
        RetryTiming {
            remaining_retries,
            max_retries,
        },
        &availability(0, 2),
        None,
        &mut random,
    );
    assert_eq!(seconds(delay), expected);
}

#[test]
fn an_enormous_exponent_stays_at_the_maximum() {
    let mut random = FixedSample::new(0);
    let delay = sleep_before_retry(
        &settings(0.0),
        RetryTiming {
            remaining_retries: 0,
            max_retries: i64::MAX,
        },
        &availability(0, 2),
        None,
        &mut random,
    );
    assert_eq!(seconds(delay), "8.000000");
}

#[test]
fn the_minimum_timeout_never_wins_over_the_maximum_delay() {
    let mut random = FixedSample::new(0);
    let delay = sleep_before_retry(
        &settings(20.0),
        RetryTiming {
            remaining_retries: 0,
            max_retries: 0,
        },
        &availability(0, 2),
        None,
        &mut random,
    );
    assert_eq!(seconds(delay), "8.000000");
}

#[rstest]
#[case::no_deployments(1, 0, "0.000000")]
#[case::one_of_two(1, 2, "0.000000")]
#[case::single_deployment(1, 1, "0.500000")]
#[case::none_healthy(0, 2, "0.500000")]
fn the_deployment_counts_decide_the_shortcut(
    #[case] healthy: usize,
    #[case] all: usize,
    #[case] expected: &str,
) {
    let mut random = FixedSample::new(0);
    let delay = sleep_before_retry(
        &settings(0.0),
        RetryTiming {
            remaining_retries: 2,
            max_retries: 2,
        },
        &availability(healthy, all),
        None,
        &mut random,
    );
    assert_eq!(seconds(delay), expected);
}

#[test]
fn the_backoff_defaults_are_litellms_constants() {
    let defaults = settings(0.0);
    assert_eq!(seconds(defaults.initial_retry_delay), "0.500000");
    assert_eq!(seconds(defaults.max_retry_delay), "8.000000");
    assert_eq!(seconds(defaults.jitter), "0.750000");
    assert_eq!(seconds(defaults.min_timeout), "0.000000");
}

#[rstest]
#[case::authentication_status_skips(401, &["dep-1"])]
#[case::forbidden_status_skips(403, &["dep-1"])]
#[case::bad_request_status_skips(400, &["dep-1"])]
#[case::not_found_status_skips(404, &["dep-1"])]
#[case::rate_limit_status_retries(429, &[])]
#[case::server_status_retries(503, &[])]
fn only_a_non_retryable_answer_skips_the_deployment(#[case] status: u16, #[case] want: &[&str]) {
    let skipped = skipped_deployment_ids(
        status,
        FailedDeployment {
            retry_skip_deployment_id: Some("dep-1"),
            failed_deployment_id: None,
        },
        &[],
    );
    let expected: Vec<String> = want.iter().map(|name| (*name).to_owned()).collect();
    assert_eq!(skipped, expected);
}

#[test]
fn skipped_deployment_ids_are_sorted_and_unique() {
    let already = ["dep-9".to_owned(), "dep-1".to_owned(), "dep-5".to_owned()];
    let skipped = skipped_deployment_ids(
        400,
        FailedDeployment {
            retry_skip_deployment_id: Some("dep-5"),
            failed_deployment_id: Some("dep-1"),
        },
        &already,
    );
    let expected = ["dep-1", "dep-5", "dep-9"].map(str::to_owned).to_vec();
    assert_eq!(skipped, expected);
}

#[test]
fn a_config_policy_converts_field_by_field() {
    let config = RetryPolicySettings {
        bad_request: Some(1),
        authentication: Some(2),
        timeout: Some(3),
        rate_limit: Some(4),
        content_policy: Some(5),
        internal_server: Some(6),
        service_unavailable: Some(7),
        not_found: Some(8),
        fallback: Some(9),
    };
    assert_eq!(RetryPolicy::from(&config), every_field());
}
