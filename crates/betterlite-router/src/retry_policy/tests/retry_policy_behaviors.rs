use std::collections::BTreeMap;

use rstest::rstest;

use super::{FixedSample, availability, every_field, seconds, settings};
use crate::cooldown_decisions::failure::FailureClass;
use crate::retry_policy::backoff::{RetryTiming, sleep_before_retry};
use crate::retry_policy::eligibility::{Availability, RetryDecision, StopReason, decide};
use crate::retry_policy::policy::{RetryCounts, RetryPolicies, RetryPolicy, plan};
use crate::retry_policy::retry_after::{RetryAfter, parse_retry_after};
use crate::retry_policy::skip::{FailedDeployment, skipped_deployment_ids};

fn with_fallbacks(
    healthy: usize,
    all: usize,
    context_window: Option<usize>,
    content_policy: Option<usize>,
    regular: Option<usize>,
) -> Availability {
    Availability {
        healthy_deployments: healthy,
        all_deployments: all,
        context_window_fallbacks: context_window,
        content_policy_fallbacks: content_policy,
        regular_fallbacks: regular,
    }
}

fn rate_limit_policies(retries: Option<i64>) -> RetryPolicies {
    RetryPolicies {
        global: Some(RetryPolicy {
            bad_request: None,
            authentication: None,
            timeout: None,
            rate_limit: retries,
            content_policy: None,
            internal_server: None,
            service_unavailable: None,
            not_found: None,
            fallback: None,
        }),
        model_group: BTreeMap::new(),
    }
}

fn selected_retries(
    policies: &RetryPolicies,
    class: FailureClass,
    status: Option<u16>,
    group: Option<&str>,
) -> Option<i64> {
    policies.selected(group)?.retries_for(class, status)
}

#[rstest]
#[case::authentication(FailureClass::Authentication, 2)]
#[case::timeout(FailureClass::Timeout, 3)]
#[case::rate_limit(FailureClass::RateLimit, 4)]
#[case::content_policy(FailureClass::ContentPolicyViolation, 5)]
#[case::bad_request(FailureClass::BadRequest, 1)]
#[case::context_window_inherits_bad_request(FailureClass::ContextWindowExceeded, 1)]
#[case::unsupported_params_inherits_bad_request(FailureClass::UnsupportedParams, 1)]
#[case::service_unavailable(FailureClass::ServiceUnavailable, 7)]
#[case::mid_stream_inherits_service_unavailable(FailureClass::MidStreamFallback, 7)]
#[case::internal_server(FailureClass::InternalServer, 6)]
#[case::not_found_without_404(FailureClass::NotFound, 9)]
#[case::permission_denied(FailureClass::PermissionDenied, 9)]
#[case::unprocessable_entity(FailureClass::UnprocessableEntity, 9)]
#[case::budget_exceeded(FailureClass::BudgetExceeded, 9)]
#[case::bad_gateway(FailureClass::BadGateway, 9)]
#[case::api(FailureClass::Api, 9)]
#[case::api_connection(FailureClass::ApiConnection, 9)]
#[case::api_response_validation(FailureClass::ApiResponseValidation, 9)]
#[case::json_schema_validation(FailureClass::JsonSchemaValidation, 9)]
#[case::guardrail_raised(FailureClass::GuardrailRaised, 9)]
#[case::unclassified(FailureClass::Unclassified, 9)]
fn a_class_takes_its_own_policy_field(#[case] class: FailureClass, #[case] expected: i64) {
    assert_eq!(every_field().retries_for(class, Some(500)), Some(expected));
}

#[rstest]
#[case::not_found_class(FailureClass::NotFound, Some(404), Some(8))]
#[case::bad_request_with_404(FailureClass::BadRequest, Some(404), Some(8))]
#[case::bad_request_keeps_bad_request(FailureClass::BadRequest, Some(400), Some(1))]
fn a_404_answer_checks_not_found_first(
    #[case] class: FailureClass,
    #[case] status: Option<u16>,
    #[case] expected: Option<i64>,
) {
    assert_eq!(every_field().retries_for(class, status), expected);
}

#[test]
fn a_404_answer_without_not_found_walks_the_class_chain() {
    let policy = RetryPolicy {
        bad_request: Some(1),
        authentication: None,
        timeout: None,
        rate_limit: None,
        content_policy: None,
        internal_server: None,
        service_unavailable: None,
        not_found: None,
        fallback: Some(9),
    };
    assert_eq!(
        policy.retries_for(FailureClass::BadRequest, Some(404)),
        Some(1)
    );
}

#[test]
fn a_model_group_policy_replaces_the_global_one_whole() {
    let global = RetryPolicy {
        bad_request: None,
        authentication: Some(6),
        timeout: None,
        rate_limit: Some(1),
        content_policy: None,
        internal_server: None,
        service_unavailable: None,
        not_found: None,
        fallback: Some(9),
    };
    let group = RetryPolicy {
        bad_request: None,
        authentication: None,
        timeout: None,
        rate_limit: None,
        content_policy: None,
        internal_server: None,
        service_unavailable: None,
        not_found: None,
        fallback: Some(2),
    };
    let mut model_group = BTreeMap::new();
    model_group.insert("gpt-4".to_owned(), group);
    let policies = RetryPolicies {
        global: Some(global),
        model_group,
    };
    assert_eq!(
        selected_retries(&policies, FailureClass::RateLimit, Some(429), Some("gpt-4")),
        Some(2)
    );
    assert_eq!(
        selected_retries(
            &policies,
            FailureClass::Authentication,
            Some(401),
            Some("gpt-4")
        ),
        Some(2)
    );
    assert_eq!(
        selected_retries(&policies, FailureClass::RateLimit, Some(429), Some("other")),
        Some(1)
    );
    assert_eq!(
        selected_retries(&policies, FailureClass::RateLimit, Some(429), None),
        Some(1)
    );
    assert_eq!(
        selected_retries(
            &RetryPolicies::default(),
            FailureClass::RateLimit,
            Some(429),
            None
        ),
        None
    );
}

#[rstest]
#[case::request_wins(Some(5), 0, Some(2), None, (5, false))]
#[case::deployment_beats_router(None, 3, Some(2), None, (2, false))]
#[case::router_without_deployment(None, 3, None, None, (3, false))]
#[case::request_zero_stops_policy(Some(0), 3, Some(2), Some(4), (0, false))]
#[case::policy_overrides_request(Some(5), 0, None, Some(4), (4, true))]
#[case::policy_overrides_router(None, 3, None, Some(4), (4, true))]
#[case::deployment_kept_when_no_policy_matches(None, 3, Some(2), None, (2, false))]
fn the_count_follows_request_deployment_and_policy(
    #[case] request: Option<i64>,
    #[case] router: i64,
    #[case] deployment: Option<u32>,
    #[case] retries: Option<i64>,
    #[case] expected: (i64, bool),
) {
    let policies = match retries {
        Some(retries) => rate_limit_policies(Some(retries)),
        None => RetryPolicies::default(),
    };
    let counts = RetryCounts {
        request,
        router,
        deployment,
    };
    let outcome = plan(counts, &policies, FailureClass::RateLimit, Some(429), None);
    assert_eq!((outcome.num_retries, outcome.policy_applies), expected);
}

#[rstest]
#[case::healthy_deployment(FailureClass::RateLimit, Some(429), availability(2, 3), true)]
#[case::auth_single_deployment(FailureClass::Authentication, Some(401), availability(1, 1), false)]
#[case::auth_two_deployments(FailureClass::Authentication, Some(401), availability(2, 2), true)]
#[case::permission_denied_two_deployments(
    FailureClass::PermissionDenied,
    Some(403),
    availability(2, 2),
    true
)]
#[case::not_found(FailureClass::NotFound, Some(404), availability(2, 2), false)]
#[case::bad_request(FailureClass::BadRequest, Some(400), availability(2, 2), false)]
#[case::internal_server(FailureClass::InternalServer, Some(500), availability(2, 2), true)]
#[case::no_healthy_deployments(FailureClass::Timeout, Some(408), availability(0, 2), false)]
#[case::timeout_with_one_deployment(FailureClass::Timeout, Some(408), availability(1, 1), true)]
fn eligibility_follows_class_status_and_candidates(
    #[case] class: FailureClass,
    #[case] status: Option<u16>,
    #[case] availability: Availability,
    #[case] retry: bool,
) {
    match decide(class, status, &availability) {
        RetryDecision::Retry => assert!(retry),
        RetryDecision::Stop(reason) => assert!(!retry, "stopped with {reason:?}"),
    }
}

#[rstest]
#[case::context_window_with_fallbacks_list(
    FailureClass::ContextWindowExceeded,
    Some(500),
    with_fallbacks(2, 2, Some(0), None, None),
    Some(StopReason::ContextWindowFallbacks)
)]
#[case::context_window_without_fallbacks(
    FailureClass::ContextWindowExceeded,
    Some(500),
    with_fallbacks(2, 2, None, None, None),
    None
)]
#[case::content_policy_with_fallbacks_list(
    FailureClass::ContentPolicyViolation,
    Some(500),
    with_fallbacks(2, 2, None, Some(0), None),
    Some(StopReason::ContentPolicyFallbacks)
)]
#[case::rate_limit_without_healthy_but_with_fallbacks(
    FailureClass::RateLimit,
    Some(429),
    with_fallbacks(0, 2, None, None, Some(1)),
    Some(StopReason::RateLimitWithoutHealthy)
)]
#[case::rate_limit_with_fallbacks_and_healthy(
    FailureClass::RateLimit,
    Some(429),
    with_fallbacks(2, 2, None, None, Some(1)),
    None
)]
#[case::rate_limit_with_empty_fallbacks_and_no_healthy(
    FailureClass::RateLimit,
    Some(429),
    with_fallbacks(0, 2, None, None, Some(0)),
    Some(StopReason::NoHealthyDeployments)
)]
#[case::rate_limit_with_empty_fallback_list(
    FailureClass::RateLimit,
    Some(429),
    with_fallbacks(2, 2, None, None, Some(0)),
    None
)]
fn fallback_lists_change_eligibility(
    #[case] class: FailureClass,
    #[case] status: Option<u16>,
    #[case] availability: Availability,
    #[case] stop: Option<StopReason>,
) {
    let expected = stop.map_or(RetryDecision::Retry, RetryDecision::Stop);
    assert_eq!(decide(class, status, &availability), expected);
}

struct SleepCase {
    all: usize,
    healthy: usize,
    retry_after: Option<i64>,
    max_retries: i64,
    remaining_retries: i64,
    min_timeout: f64,
    expected: &'static str,
}

#[rstest]
#[case::healthy_alternative(SleepCase { all: 2, healthy: 1, retry_after: None, max_retries: 2, remaining_retries: 2, min_timeout: 0.0, expected: "0.000000" })]
#[case::first_retry(SleepCase { all: 2, healthy: 0, retry_after: None, max_retries: 2, remaining_retries: 2, min_timeout: 0.0, expected: "0.500000" })]
#[case::second_retry(SleepCase { all: 2, healthy: 0, retry_after: None, max_retries: 2, remaining_retries: 1, min_timeout: 0.0, expected: "1.000000" })]
#[case::requested_retry_after(SleepCase { all: 2, healthy: 0, retry_after: Some(5), max_retries: 2, remaining_retries: 2, min_timeout: 0.0, expected: "5.000000" })]
#[case::retry_after_over_60(SleepCase { all: 2, healthy: 0, retry_after: Some(61), max_retries: 2, remaining_retries: 2, min_timeout: 0.0, expected: "0.500000" })]
#[case::minimum_timeout_raises(SleepCase { all: 2, healthy: 0, retry_after: None, max_retries: 2, remaining_retries: 2, min_timeout: 3.0, expected: "3.000000" })]
#[case::single_deployment_backs_off(SleepCase { all: 1, healthy: 1, retry_after: None, max_retries: 2, remaining_retries: 2, min_timeout: 0.0, expected: "0.500000" })]
#[case::no_healthy_deployments(SleepCase { all: 2, healthy: 0, retry_after: None, max_retries: 2, remaining_retries: 2, min_timeout: 0.0, expected: "0.500000" })]
fn the_sleep_follows_candidates_and_the_header(#[case] case: SleepCase) {
    let mut random = FixedSample::new(0);
    let delay = sleep_before_retry(
        &settings(case.min_timeout),
        RetryTiming {
            remaining_retries: case.remaining_retries,
            max_retries: case.max_retries,
        },
        &availability(case.healthy, case.all),
        case.retry_after,
        &mut random,
    );
    assert_eq!(seconds(delay), case.expected);
}

#[test]
fn a_healthy_alternative_does_not_draw_a_sample() {
    let mut random = FixedSample::new(0);
    let delay = sleep_before_retry(
        &settings(0.0),
        RetryTiming {
            remaining_retries: 2,
            max_retries: 2,
        },
        &availability(1, 2),
        None,
        &mut random,
    );
    assert_eq!(seconds(delay), "0.000000");
    assert_eq!(random.draws, 0);
}

#[test]
fn the_jitter_scales_the_injected_sample() {
    let mut random = FixedSample::new(4_503_599_627_370_496);
    let delay = sleep_before_retry(
        &settings(0.0),
        RetryTiming {
            remaining_retries: 2,
            max_retries: 2,
        },
        &availability(0, 2),
        None,
        &mut random,
    );
    assert_eq!(seconds(delay), "0.875000");
}

#[rstest]
#[case::plain_seconds(b"7", RetryAfter::Seconds(7))]
#[case::surrounding_spaces(b" 7 ", RetryAfter::Seconds(7))]
#[case::plus_sign(b"+7", RetryAfter::Seconds(7))]
#[case::negative(b"-7", RetryAfter::Seconds(-7))]
#[case::leading_zeroes(b"007", RetryAfter::Seconds(7))]
#[case::http_date(b"Wed, 21 Oct 2015 07:28:00 GMT", RetryAfter::At { unix_millis: 1_445_412_480_000 })]
fn a_retry_after_value_is_seconds_or_a_date(#[case] bytes: &[u8], #[case] expected: RetryAfter) {
    assert_eq!(parse_retry_after(bytes).unwrap(), expected);
}

#[rstest]
#[case::rfc_850(b"Sunday, 06-Nov-94 08:49:37 GMT", 784_111_777)]
#[case::asctime(b"Sun Nov  6 08:49:37 1994", 784_111_777)]
fn the_two_legacy_http_date_formats_parse(#[case] bytes: &[u8], #[case] expected: i64) {
    assert_eq!(
        parse_retry_after(bytes).unwrap(),
        RetryAfter::At {
            unix_millis: expected.saturating_mul(1000)
        }
    );
}

#[rstest]
#[case::retry_skip_wins(
    400,
    Some("dep-1"),
    Some("dep-2"),
    &["dep-0"],
    &["dep-0", "dep-1"]
)]
#[case::failed_deployment_used(400, None, Some("dep-2"), &["dep-2"], &["dep-2"])]
#[case::retryable_status_skips_nothing(500, Some("dep-1"), None, &[], &[])]
#[case::no_deployment_skips_nothing(400, None, None, &["dep-0"], &[])]
fn a_non_retryable_answer_skips_its_deployment(
    #[case] status: u16,
    #[case] retry_skip: Option<&str>,
    #[case] failed: Option<&str>,
    #[case] already_skipped: &[&str],
    #[case] want: &[&str],
) {
    let already: Vec<String> = already_skipped
        .iter()
        .map(|name| (*name).to_owned())
        .collect();
    let expected: Vec<String> = want.iter().map(|name| (*name).to_owned()).collect();
    let skipped = skipped_deployment_ids(
        status,
        FailedDeployment {
            retry_skip_deployment_id: retry_skip,
            failed_deployment_id: failed,
        },
        &already,
    );
    assert_eq!(skipped, expected);
}
