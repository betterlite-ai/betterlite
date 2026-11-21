use std::collections::BTreeMap;
use std::iter::repeat_with;
use std::time::Duration;

use betterlite_error::error::ErrorClass;
use betterlite_error::error::details::{
    BudgetExceededDetails, ResponseDetails, TimeoutDetails, TimeoutStatus,
};
use rstest::rstest;
use rust_decimal::Decimal;

use super::{
    Driven, Scenario, Step, authentication, availability, bad_request, context, drive, guardrail,
    internal, millis, rate_limit,
};
use crate::cooldown_decisions::failure::FailureClass;
use crate::retry_policy::policy::{RetryPolicies, RetryPolicy};
use crate::router_core::error::RoutingError;
use crate::routing_retries::error::{RetryError, exhausted};
use crate::routing_timeouts::error::TimeoutError;

fn provider_error(driven: Driven) -> ErrorClass {
    match driven.retried.outcome {
        Err(RetryError::Provider(error)) => *error,
        Ok(answer) => panic!("expected a provider error, got {answer}"),
        Err(other) => panic!("expected a provider error, got {other:?}"),
    }
}

fn timeout_error(driven: &Driven) -> &TimeoutError {
    match &driven.retried.outcome {
        Err(RetryError::Timeout(error)) => error,
        Ok(answer) => panic!("expected a timeout, got {answer}"),
        Err(other) => panic!("expected a timeout, got {other:?}"),
    }
}

fn internal_policy(retries: i64) -> RetryPolicies {
    RetryPolicies {
        global: Some(RetryPolicy {
            bad_request: None,
            authentication: None,
            timeout: None,
            rate_limit: None,
            content_policy: None,
            internal_server: Some(retries),
            service_unavailable: None,
            not_found: None,
            fallback: None,
        }),
        model_group: BTreeMap::new(),
    }
}

fn stamped(message: &str, retries: u32) -> ErrorClass {
    let mut counted = context(message);
    counted.max_retries = Some(retries);
    counted.num_retries = Some(retries);
    ErrorClass::InternalServer(Box::new(counted))
}

#[tokio::test(start_paused = true)]
async fn exhausted_retries_raise_the_latest_error_with_its_counts() {
    let scenario = Scenario::new(
        vec![
            Step::Fail("a", internal("one")),
            Step::Fail("a", internal("two")),
            Step::Fail("a", internal("three")),
        ],
        2,
    );
    let driven = drive(scenario).await;
    assert_eq!(driven.calls.len(), 3);
    assert_eq!(driven.retried.trail.attempted_retries, 2);
    assert_eq!(driven.retried.trail.backoffs, millis(&[500, 1000]));
    assert_eq!(driven.elapsed, Duration::from_millis(1500));
    assert_eq!(provider_error(driven), stamped("three", 2));
}

fn refused(scenario: Scenario, seen: (usize, usize)) -> Scenario {
    let mut changed = scenario;
    changed.availability = vec![availability(seen.0, seen.1)];
    changed
}

fn with_request(scenario: Scenario, request: i64, policies: RetryPolicies) -> Scenario {
    let mut changed = scenario;
    changed.request_num_retries = Some(request);
    changed.policies = policies;
    changed
}

// (provider calls, max retries the trail reports, breadcrumbs)
#[rstest]
#[case::second_error_not_retryable(
    Scenario::new(vec![Step::Fail("a", rate_limit("busy")), Step::Fail("a", bad_request("bad"))], 2),
    bad_request("bad"),
    (2, 2, 2)
)]
#[case::request_zero_disables_the_policy(
    with_request(Scenario::new(vec![Step::Fail("a", internal("down"))], 2), 0, internal_policy(3)),
    internal("down"),
    (1, 0, 0)
)]
#[case::router_zero_never_retries(Scenario::new(vec![Step::Fail("a", internal("down"))], 0), internal("down"), (1, 0, 0))]
#[case::guardrail_block_is_never_retried(Scenario::new(vec![Step::Fail("a", guardrail("blocked"))], 2), guardrail("blocked"), (1, 2, 0))]
#[case::no_healthy_deployment(
    refused(Scenario::new(vec![Step::Fail("a", internal("down"))], 2), (0, 1)),
    internal("down"),
    (1, 2, 0)
)]
#[case::single_deployment_auth_error(
    Scenario::new(vec![Step::Fail("a", authentication("key"))], 2),
    authentication("key"),
    (1, 2, 0)
)]
#[tokio::test(start_paused = true)]
async fn refused_errors_return_without_counts(
    #[case] scenario: Scenario,
    #[case] expected: ErrorClass,
    #[case] seen: (usize, usize, usize),
) {
    let driven = drive(scenario).await;
    assert_eq!(driven.calls.len(), seen.0);
    assert_eq!(
        driven.retried.trail.max_retries,
        i64::try_from(seen.1).unwrap()
    );
    assert_eq!(driven.retried.trail.previous_models.len(), seen.2);
    assert_eq!(provider_error(driven), expected);
}

#[rstest]
#[case::backoff_reaching_the_deadline_stops(Duration::from_millis(500), 1)]
#[case::backoff_inside_the_deadline_retries(Duration::from_millis(501), 2)]
#[tokio::test(start_paused = true)]
async fn backoff_obeys_the_routing_deadline(#[case] deadline: Duration, #[case] calls: usize) {
    let mut scenario = Scenario::new(
        vec![Step::Fail("a", rate_limit("busy")), Step::Answer("ok")],
        2,
    );
    scenario.deadline = deadline;
    let driven = drive(scenario).await;
    assert_eq!(driven.calls.len(), calls);
    if calls == 1 {
        assert!(driven.retried.trail.backoffs.is_empty());
        assert_eq!(provider_error(driven), rate_limit("busy"));
    } else {
        assert_eq!(driven.retried.outcome.unwrap(), "ok");
    }
}

#[rstest]
#[case::disconnect_while_the_attempt_fails(
    Scenario::new(vec![Step::Disconnect("a", rate_limit("busy")), Step::Answer("ok")], 2),
    TimeoutError::Cancelled,
    Duration::ZERO
)]
#[case::attempt_outlives_the_deadline(Scenario::new(vec![Step::Hang, Step::Answer("ok")], 2), TimeoutError::DeadlineExpired, Duration::from_secs(30))]
#[tokio::test(start_paused = true)]
async fn abandoned_attempts_cancel_their_transport(
    #[case] scenario: Scenario,
    #[case] expected: TimeoutError,
    #[case] elapsed: Duration,
) {
    let driven = drive(scenario).await;
    assert_eq!(driven.calls.len(), 1);
    assert_eq!(timeout_error(&driven), &expected);
    assert_eq!(driven.elapsed, elapsed);
    assert!(driven.attempt_tokens[0].is_cancelled());
}

#[rstest]
#[case::during_the_backoff(Step::Fail("a", rate_limit("busy")))]
#[case::during_the_attempt(Step::Hang)]
#[tokio::test(start_paused = true)]
async fn disconnect_stops_at_once(#[case] first: Step) {
    let mut scenario = Scenario::new(vec![first, Step::Answer("ok")], 2);
    scenario.cancel_after = Some(Duration::from_millis(100));
    let driven = drive(scenario).await;
    assert_eq!(driven.calls.len(), 1);
    assert_eq!(timeout_error(&driven), &TimeoutError::Cancelled);
    assert_eq!(driven.elapsed, Duration::from_millis(100));
    assert!(driven.attempt_tokens[0].is_cancelled());
}

#[tokio::test(start_paused = true)]
async fn unroutable_attempt_is_not_retried() {
    let driven = drive(Scenario::new(vec![Step::Unroutable, Step::Answer("ok")], 2)).await;
    assert_eq!(driven.calls.len(), 1);
    let Err(RetryError::Routing(error)) = &driven.retried.outcome else {
        panic!(
            "expected the routing error, got {:?}",
            driven.retried.outcome
        );
    };
    assert!(matches!(**error, RoutingError::Cancelled));
}

#[tokio::test(start_paused = true)]
async fn non_retryable_answers_skip_their_deployment() {
    let mut scenario = Scenario::new(
        vec![
            Step::Fail("a", authentication("key")),
            Step::Fail("b", authentication("key")),
            Step::Fail("a", internal("down")),
            Step::Answer("ok"),
        ],
        3,
    );
    scenario.availability = vec![availability(1, 2)];
    let driven = drive(scenario).await;
    let skipped: Vec<_> = driven
        .calls
        .iter()
        .map(|call| call.skipped.clone())
        .collect();
    let both = vec!["a".to_owned(), "b".to_owned()];
    assert_eq!(
        skipped,
        vec![vec![], vec!["a".to_owned()], both.clone(), both]
    );
}

#[tokio::test(start_paused = true)]
async fn breadcrumbs_keep_the_latest_four() {
    let steps = repeat_with(|| Step::Fail("a", internal("down")))
        .take(5)
        .collect();
    let driven = drive(Scenario::new(steps, 4)).await;
    let trail = &driven.retried.trail;
    let attempts: Vec<_> = trail
        .previous_models
        .iter()
        .map(|attempt| attempt.attempted_retries)
        .collect();
    assert_eq!(attempts, vec![1, 2, 3, 4]);
    assert_eq!(trail.request_retry_count, 5);
    assert_eq!(driven.calls.len(), 5);
}

#[tokio::test(start_paused = true)]
async fn plain_exception_has_no_status() {
    let plain = ErrorClass::Unclassified(Box::new(context("boom")));
    let driven = drive(Scenario::new(
        vec![Step::Fail("a", plain), Step::Answer("ok")],
        1,
    ))
    .await;
    let attempt = &driven.retried.trail.previous_models[0];
    assert_eq!(attempt.class, FailureClass::Unclassified);
    assert_eq!(attempt.status, None);
    assert!(driven.calls[1].skipped.is_empty());
}

fn timeout() -> ErrorClass {
    ErrorClass::Timeout(Box::new(TimeoutDetails {
        context: context("slow"),
        original_status: TimeoutStatus(408),
    }))
}

fn budget() -> ErrorClass {
    ErrorClass::BudgetExceeded(Box::new(BudgetExceededDetails {
        current_cost: Decimal::ONE,
        max_budget: Decimal::ONE,
        message: None,
        provider: String::new(),
        entity_type: None,
        entity_id: None,
        status_code: 400,
    }))
}

fn counted_bad_request() -> ErrorClass {
    let mut details = ResponseDetails::with_status(context("bad"), Some(400));
    details.max_retries = Some(3);
    details.num_retries = Some(3);
    ErrorClass::BadRequest(Box::new(details))
}

fn counted_timeout() -> ErrorClass {
    let mut counted = context("slow");
    counted.max_retries = Some(3);
    counted.num_retries = Some(3);
    ErrorClass::Timeout(Box::new(TimeoutDetails {
        context: counted,
        original_status: TimeoutStatus(408),
    }))
}

#[rstest]
#[case::response_family(bad_request("bad"), counted_bad_request())]
#[case::context_family(internal("x"), stamped("x", 3))]
#[case::nested_context(timeout(), counted_timeout())]
#[case::budget_is_no_litellm_exception_type(budget(), budget())]
#[case::guardrail_is_no_litellm_exception_type(guardrail("g"), guardrail("g"))]
fn exhaustion_stamps_only_litellm_exception_types(
    #[case] error: ErrorClass,
    #[case] expected: ErrorClass,
) {
    let RetryError::Provider(stamped_error) = exhausted(error, 3) else {
        panic!("exhaustion answers the provider error");
    };
    assert_eq!(*stamped_error, expected);
}
