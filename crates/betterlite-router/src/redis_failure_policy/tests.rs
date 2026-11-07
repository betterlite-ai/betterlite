#![cfg(test)]

use std::future::Future;
use std::pin::Pin;
use std::sync::Mutex;
use std::task::{Context, Poll, Waker};
use std::time::Duration;

use betterlite_redis::breaker_metrics::BreakerMetrics;
use prometheus_client::encoding::text::encode;
use prometheus_client::registry::Registry;
use rstest::rstest;

use super::classify::{Failure, FailureClass, FailureKind, failure_kind, is_timeout_failure};
use super::log::{LogDecision, TimeoutLogThrottle};
use super::run::{RedisCallError, run_async};
use super::{BreakerEvent, RedisFailurePolicy, Settings, State, Status};

const RECOVERY_SECS: u64 = 30;

fn millis(value: u64) -> Duration {
    Duration::from_millis(value)
}

fn secs(value: u64) -> Duration {
    Duration::from_secs(value)
}

fn settings(failure_threshold: u32, enabled: bool) -> Settings {
    Settings {
        enabled,
        failure_threshold,
        recovery_timeout: secs(RECOVERY_SECS),
        timeout_min_duration: millis(5_000),
        timeout_log_interval: secs(1),
    }
}

fn status(
    state: State,
    generation: u64,
    failure_count: u32,
    hard_failure_count: u32,
    timeout_count: u32,
) -> Status {
    Status {
        state,
        generation,
        failure_count,
        hard_failure_count,
        timeout_count,
    }
}

fn failure(class: FailureClass) -> Failure<'static> {
    Failure { class, causes: &[] }
}

fn opened(failure_count: u32, hard_failure_count: u32) -> BreakerEvent {
    BreakerEvent::Opened {
        failure_count,
        hard_failure_count,
        recovery_timeout: secs(RECOVERY_SECS),
    }
}

fn fail(policy: &mut RedisFailurePolicy, now: u64, class: FailureClass) -> Option<BreakerEvent> {
    policy.record_failure(millis(now), failure(class))
}

fn timeout_fail(policy: &mut RedisFailurePolicy, now: u64) -> Option<BreakerEvent> {
    fail(policy, now, FailureClass::Timeout)
}

fn policy(settings: Settings) -> RedisFailurePolicy {
    RedisFailurePolicy::new(settings, BreakerMetrics::new(None))
}

fn opened_policy(configuration: Settings) -> RedisFailurePolicy {
    let mut policy = policy(configuration);
    fail(&mut policy, 0, FailureClass::Connection);
    policy
}

#[rstest]
#[case::timeout_streak_opens_after_the_threshold_spans_the_minimum(timeout_streak_opens)]
#[case::timeout_streak_below_the_minimum_duration_stays_closed(timeout_streak_below_the_minimum)]
#[case::connectivity_failures_open_at_the_threshold(connectivity_opens_at_the_threshold)]
#[case::a_connectivity_failure_restarts_the_timeout_streak(
    a_connectivity_failure_restarts_the_streak
)]
#[case::failures_while_open_warn_only_once(failures_while_open_warn_once)]
#[case::a_success_while_open_is_ignored(a_success_while_open_is_ignored)]
#[case::a_streak_below_the_threshold_never_opens(a_streak_below_the_threshold_never_opens)]
#[case::a_disabled_policy_never_counts_or_opens(a_disabled_policy_never_counts)]
#[case::a_success_resets_the_counts(a_success_resets_the_counts)]
#[case::a_half_open_success_closes_and_reports_recovery(half_open_success_closes)]
#[case::a_swallowed_failure_keeps_the_call_from_closing(a_swallowed_failure_keeps_the_call)]
#[case::a_swallowed_non_health_failure_does_not_mark_the_call(
    a_swallowed_non_health_failure_does_not_mark_the_call
)]
#[case::an_explicit_timeout_cause_makes_a_connection_failure_a_timeout(
    an_explicit_timeout_cause_is_a_timeout
)]
#[case::timeout_logs_are_throttled_and_counted(timeout_logs_are_throttled)]
#[case::circuit_open_and_command_failures_do_not_spend_the_log_interval(
    circuit_open_and_command_failures_do_not_spend_the_log_interval
)]
#[case::metrics_record_the_initial_state_failures_and_transitions(
    metrics_record_states_failures_and_transitions
)]
fn redis_failure_policy_behaviors(#[case] scenario: fn()) {
    scenario();
}

fn timeout_streak_opens() {
    let mut policy = policy(settings(3, true));
    let events = [
        timeout_fail(&mut policy, 1_000),
        timeout_fail(&mut policy, 2_000),
        timeout_fail(&mut policy, 4_000),
        timeout_fail(&mut policy, 5_999),
        timeout_fail(&mut policy, 6_000),
    ];
    assert_eq!(
        events,
        [None, None, None, None, Some(opened(5, 0))],
        "only the failure that spans the minimum duration opens the breaker"
    );
    assert_eq!(policy.status(), status(State::Open, 1, 5, 0, 5));
}

fn timeout_streak_below_the_minimum() {
    let mut policy = policy(settings(3, true));
    let events = [
        timeout_fail(&mut policy, 1_000),
        timeout_fail(&mut policy, 2_000),
        timeout_fail(&mut policy, 4_000),
        timeout_fail(&mut policy, 5_000),
    ];
    assert_eq!(events, [None, None, None, None]);
    assert_eq!(policy.status(), status(State::Closed, 0, 4, 0, 4));
}

fn connectivity_opens_at_the_threshold() {
    let mut policy = policy(settings(3, true));
    let events = [
        fail(&mut policy, 0, FailureClass::Connection),
        fail(&mut policy, 10, FailureClass::Connection),
        fail(&mut policy, 20, FailureClass::Connection),
    ];
    assert_eq!(events, [None, None, Some(opened(3, 3))]);
    assert_eq!(policy.status(), status(State::Open, 1, 3, 3, 0));
}

fn a_connectivity_failure_restarts_the_streak() {
    let mut policy = policy(settings(3, true));
    timeout_fail(&mut policy, 0);
    timeout_fail(&mut policy, 1_000);
    fail(&mut policy, 2_000, FailureClass::Connection);
    assert_eq!(policy.status(), status(State::Closed, 0, 3, 1, 0));
    timeout_fail(&mut policy, 3_000);
    assert_eq!(timeout_fail(&mut policy, 7_000), None);
    assert_eq!(policy.status(), status(State::Closed, 0, 5, 1, 2));
    assert_eq!(timeout_fail(&mut policy, 9_000), Some(opened(6, 1)));
    assert_eq!(policy.status(), status(State::Open, 1, 6, 1, 3));
}

fn failures_while_open_warn_once() {
    let mut policy = opened_policy(settings(1, true));
    assert_eq!(policy.status().generation, 1);
    let again = fail(&mut policy, 5, FailureClass::Connection);
    assert_eq!(again, None, "an already open breaker warns once");
    assert_eq!(policy.status(), status(State::Open, 1, 2, 2, 0));
}

fn a_success_while_open_is_ignored() {
    let mut policy = opened_policy(settings(1, true));
    assert_eq!(policy.record_success(), None);
    assert_eq!(
        policy.status(),
        status(State::Open, 1, 1, 1, 0),
        "a success cannot close an open breaker"
    );
}

fn a_streak_below_the_threshold_never_opens() {
    let mut policy = policy(settings(3, true));
    assert_eq!(timeout_fail(&mut policy, 0), None);
    assert_eq!(
        timeout_fail(&mut policy, 10_000),
        None,
        "the duration alone does not open the breaker"
    );
    assert_eq!(policy.status(), status(State::Closed, 0, 2, 0, 2));
}

fn a_disabled_policy_never_counts() {
    let mut policy = policy(settings(1, false));
    let events = [
        fail(&mut policy, 0, FailureClass::Connection),
        fail(&mut policy, 1, FailureClass::Connection),
        fail(&mut policy, 2, FailureClass::Timeout),
        fail(&mut policy, 3, FailureClass::Connection),
    ];
    assert_eq!(events, [None, None, None, None]);
    assert_eq!(policy.status(), status(State::Closed, 0, 0, 0, 0));
    let mut admission = policy
        .admit(millis(4), "get")
        .expect("a disabled breaker admits");
    let swallowed = policy.record_swallowed_failure(
        &mut admission,
        millis(5),
        failure(FailureClass::Connection),
    );
    assert_eq!(swallowed, None);
    assert_eq!(
        policy.status(),
        status(State::Closed, 0, 0, 0, 0),
        "a swallowed failure on a disabled breaker is not counted"
    );
}

fn a_success_resets_the_counts() {
    let mut policy = policy(settings(3, true));
    timeout_fail(&mut policy, 0);
    timeout_fail(&mut policy, 1_000);
    let admission = policy
        .admit(millis(2_000), "get")
        .expect("the breaker is closed");
    assert_eq!(policy.finish(admission), None);
    assert_eq!(policy.status(), status(State::Closed, 0, 0, 0, 0));
}

fn half_open_success_closes() {
    let mut policy = opened_policy(settings(1, true));
    let admission = policy
        .admit(millis(30_001), "get")
        .expect("the designated probe is admitted");
    assert_eq!(policy.status(), status(State::HalfOpen, 2, 1, 1, 0));
    assert_eq!(policy.finish(admission), Some(BreakerEvent::Recovered));
    assert_eq!(policy.status(), status(State::Closed, 3, 0, 0, 0));
}

fn a_swallowed_failure_keeps_the_call() {
    let mut policy = policy(settings(3, true));
    let mut admission = policy
        .admit(millis(0), "get")
        .expect("the breaker is closed");
    let event = policy.record_swallowed_failure(
        &mut admission,
        millis(1),
        failure(FailureClass::Connection),
    );
    assert_eq!(event, None);
    assert_eq!(policy.status(), status(State::Closed, 0, 1, 1, 0));
    assert_eq!(policy.finish(admission), None);
    assert_eq!(
        policy.status(),
        status(State::Closed, 0, 1, 1, 0),
        "a call that swallowed a failure does not vouch for Redis"
    );
}

fn a_swallowed_non_health_failure_does_not_mark_the_call() {
    let mut policy = policy(settings(3, true));
    timeout_fail(&mut policy, 0);
    let mut admission = policy
        .admit(millis(1), "get")
        .expect("the breaker is closed");
    policy.record_swallowed_failure(&mut admission, millis(2), failure(FailureClass::Command));
    assert_eq!(policy.status().failure_count, 1);
    assert_eq!(policy.finish(admission), None);
    assert_eq!(
        policy.status(),
        status(State::Closed, 0, 0, 0, 0),
        "a non-health failure neither counts nor blocks the success"
    );
}

fn an_explicit_timeout_cause_is_a_timeout() {
    let mut policy = policy(settings(3, true));
    let chained = Failure {
        class: FailureClass::Connection,
        causes: &[FailureClass::Timeout],
    };
    assert_eq!(policy.record_failure(millis(0), chained), None);
    assert_eq!(policy.status(), status(State::Closed, 0, 1, 0, 1));
}

fn timeout_logs_are_throttled() {
    let mut policy = policy(settings(3, true));
    let decisions = [
        policy.log_failure(millis(0), failure(FailureClass::Timeout)),
        policy.log_failure(millis(500), failure(FailureClass::Timeout)),
        policy.log_failure(millis(900), failure(FailureClass::Timeout)),
        policy.log_failure(millis(1_000), failure(FailureClass::Timeout)),
        policy.log_failure(millis(1_500), failure(FailureClass::Timeout)),
        policy.log_failure(millis(2_000), failure(FailureClass::Timeout)),
    ];
    assert_eq!(
        decisions,
        [
            LogDecision::Level { suppressed: 0 },
            LogDecision::Debug,
            LogDecision::Debug,
            LogDecision::Level { suppressed: 2 },
            LogDecision::Debug,
            LogDecision::Level { suppressed: 1 },
        ]
    );
}

fn circuit_open_and_command_failures_do_not_spend_the_log_interval() {
    let mut policy = policy(settings(3, true));
    assert_eq!(
        policy.log_failure(millis(0), failure(FailureClass::CircuitOpen)),
        LogDecision::Debug
    );
    assert_eq!(
        policy.log_failure(millis(1), failure(FailureClass::Command)),
        LogDecision::Level { suppressed: 0 }
    );
    assert_eq!(
        policy.log_failure(millis(2), failure(FailureClass::Timeout)),
        LogDecision::Level { suppressed: 0 },
        "neither earlier failure consumed the first throttled line"
    );
}

fn metrics_record_states_failures_and_transitions() {
    let mut registry = Registry::default();
    let metrics = BreakerMetrics::new(Some(&mut registry));
    let mut policy = RedisFailurePolicy::new(settings(1, true), metrics);
    timeout_fail(&mut policy, 0);
    fail(&mut policy, 1, FailureClass::Connection);
    let mut text = String::new();
    encode(&mut text, &registry).expect("the registry encodes");
    assert!(text.contains("litellm_redis_circuit_breaker_state{state=\"closed\"} 0"));
    assert!(text.contains("litellm_redis_circuit_breaker_state{state=\"open\"} 1"));
    assert!(text.contains("litellm_redis_circuit_breaker_transitions_total{state=\"open\"} 1"));
    assert!(
        text.contains("litellm_redis_circuit_breaker_failures_total{failure_class=\"timeout\"} 1")
    );
    assert!(text.contains(
        "litellm_redis_circuit_breaker_failures_total{failure_class=\"connectivity\"} 1"
    ));
}

#[rstest]
#[case::recovery_at_exactly_the_timeout(recovery_at_exactly_the_timeout)]
#[case::recovery_one_millisecond_past_the_timeout(recovery_one_millisecond_past_the_timeout)]
#[case::timeout_streak_at_exactly_the_minimum(timeout_streak_at_exactly_the_minimum)]
#[case::timeout_streak_one_millisecond_below_the_minimum(
    timeout_streak_one_millisecond_below_the_minimum
)]
#[case::hard_failures_one_below_the_threshold(hard_failures_one_below_the_threshold)]
#[case::causes_at_the_depth_limit(causes_at_the_depth_limit)]
#[case::causes_past_the_depth_limit(causes_past_the_depth_limit)]
#[case::throttle_at_exactly_the_interval(throttle_at_exactly_the_interval)]
#[case::throttle_one_millisecond_below_the_interval(throttle_one_millisecond_below_the_interval)]
#[case::the_open_error_names_the_operation(the_open_error_names_the_operation)]
#[case::a_stale_admission_records_no_success(a_stale_admission_records_no_success)]
#[case::a_half_open_failure_reopens_without_the_hard_count(
    a_half_open_failure_reopens_without_the_hard_count
)]
#[case::a_repeated_failure_while_open_pins_the_timer(a_repeated_failure_pins_the_timer)]
#[case::a_cancelled_call_records_no_success(a_cancelled_call_records_no_success)]
fn redis_failure_policy_boundaries(#[case] boundary: fn()) {
    boundary();
}

fn recovery_at_exactly_the_timeout() {
    let mut policy = opened_policy(settings(1, true));
    let error = policy
        .admit(millis(30_000), "get_cache")
        .expect_err("the recovery window is strict");
    assert_eq!(error.operation, "get_cache");
    assert_eq!(policy.status(), status(State::Open, 1, 1, 1, 0));
}

fn recovery_one_millisecond_past_the_timeout() {
    let mut policy = opened_policy(settings(1, true));
    let admission = policy
        .admit(millis(30_001), "get_cache")
        .expect("the designated probe is admitted");
    assert_eq!(policy.status(), status(State::HalfOpen, 2, 1, 1, 0));
    assert_eq!(
        policy
            .admit(millis(30_002), "get_cache")
            .unwrap_err()
            .operation,
        "get_cache"
    );
    assert_eq!(policy.finish(admission), Some(BreakerEvent::Recovered));
}

fn timeout_streak_at_exactly_the_minimum() {
    let mut policy = policy(settings(3, true));
    timeout_fail(&mut policy, 0);
    timeout_fail(&mut policy, 0);
    assert_eq!(timeout_fail(&mut policy, 5_000), Some(opened(3, 0)));
    assert_eq!(policy.status(), status(State::Open, 1, 3, 0, 3));
}

fn timeout_streak_one_millisecond_below_the_minimum() {
    let mut policy = policy(settings(3, true));
    timeout_fail(&mut policy, 0);
    timeout_fail(&mut policy, 0);
    assert_eq!(timeout_fail(&mut policy, 4_999), None);
    assert_eq!(policy.status(), status(State::Closed, 0, 3, 0, 3));
}

fn hard_failures_one_below_the_threshold() {
    let mut policy = policy(settings(3, true));
    assert_eq!(fail(&mut policy, 0, FailureClass::Connection), None);
    assert_eq!(fail(&mut policy, 1, FailureClass::Connection), None);
    assert_eq!(policy.status(), status(State::Closed, 0, 2, 2, 0));
}

fn cause_depth(cause_count: usize, timeout: bool) {
    let mut causes = vec![FailureClass::Command; cause_count.saturating_sub(1)];
    causes.push(FailureClass::Timeout);
    assert_eq!(causes.len(), cause_count);
    assert_eq!(
        is_timeout_failure(FailureClass::Connection, &causes),
        timeout
    );
    let kind = if timeout {
        FailureKind::Timeout
    } else {
        FailureKind::Connectivity
    };
    assert_eq!(failure_kind(FailureClass::Connection, &causes), Some(kind));
}

fn causes_at_the_depth_limit() {
    cause_depth(19, true);
}

fn causes_past_the_depth_limit() {
    cause_depth(20, false);
}

fn throttle() -> TimeoutLogThrottle {
    TimeoutLogThrottle {
        interval: secs(1),
        last_logged_at: None,
        suppressed: 0,
    }
}

fn throttle_at_exactly_the_interval() {
    let mut throttle = throttle();
    assert_eq!(throttle.admit(millis(0)), Some(0));
    assert_eq!(throttle.admit(millis(1_000)), Some(0));
}

fn throttle_one_millisecond_below_the_interval() {
    let mut throttle = throttle();
    assert_eq!(throttle.admit(millis(0)), Some(0));
    assert_eq!(throttle.admit(millis(999)), None);
    assert_eq!(throttle.admit(millis(1_000)), Some(1));
}

fn the_open_error_names_the_operation() {
    let mut policy = opened_policy(settings(1, true));
    let error = policy
        .admit(millis(1), "get_cache")
        .expect_err("the breaker is open");
    assert_eq!(error.operation, "get_cache");
    assert_eq!(
        error.to_string(),
        "Redis circuit breaker is open \u{2014} skipping get_cache"
    );
}

fn a_stale_admission_records_no_success() {
    let mut policy = policy(settings(1, true));
    let stale = policy
        .admit(millis(0), "get")
        .expect("the breaker is closed");
    fail(&mut policy, 1, FailureClass::Connection);
    let probe = policy
        .admit(millis(30_002), "get")
        .expect("the designated probe is admitted");
    assert_eq!(policy.status(), status(State::HalfOpen, 2, 1, 1, 0));
    assert_eq!(policy.finish(stale), None);
    assert_eq!(
        policy.status(),
        status(State::HalfOpen, 2, 1, 1, 0),
        "a call admitted before the breaker opened cannot close it"
    );
    assert_eq!(policy.finish(probe), Some(BreakerEvent::Recovered));
    assert_eq!(policy.status(), status(State::Closed, 3, 0, 0, 0));
}

fn a_half_open_failure_reopens_without_the_hard_count() {
    let mut policy = policy(settings(3, true));
    timeout_fail(&mut policy, 1_000);
    timeout_fail(&mut policy, 2_000);
    assert_eq!(timeout_fail(&mut policy, 6_000), Some(opened(3, 0)));
    let _probe = policy
        .admit(millis(36_001), "get")
        .expect("the designated probe is admitted");
    assert_eq!(policy.status(), status(State::HalfOpen, 2, 3, 0, 3));
    assert_eq!(
        fail(&mut policy, 36_002, FailureClass::Connection),
        Some(opened(4, 1)),
        "only the half-open state reopens here, since the hard count stays below the threshold"
    );
    assert_eq!(policy.status(), status(State::Open, 3, 4, 1, 0));
}

fn a_repeated_failure_pins_the_timer() {
    let mut policy = opened_policy(settings(1, true));
    assert_eq!(fail(&mut policy, 10_000, FailureClass::Connection), None);
    assert_eq!(policy.status(), status(State::Open, 1, 2, 2, 0));
    let error = policy
        .admit(millis(40_000), "get")
        .expect_err("the window restarts at the last failure");
    assert_eq!(error.operation, "get");
    let _probe = policy
        .admit(millis(40_001), "get")
        .expect("the window has passed");
}

struct NoFailure;

impl RedisCallError for NoFailure {
    fn failure(&self) -> Option<Failure<'_>> {
        None
    }
}

struct PendingCall;

impl Future for PendingCall {
    type Output = Result<u32, NoFailure>;

    fn poll(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<Self::Output> {
        Poll::Pending
    }
}

fn cancel_after_one_poll<F>(call: F)
where
    F: Future,
{
    let mut future = Box::pin(call);
    let waker = Waker::noop();
    let mut context = Context::from_waker(waker);
    assert!(future.as_mut().poll(&mut context).is_pending());
    // the future is dropped here, as a cancelled request would be
}

fn a_cancelled_call_records_no_success() {
    let clock = || millis(10);
    let policy = Mutex::new(RedisFailurePolicy::new(
        settings(3, true),
        BreakerMetrics::new(None),
    ));
    let opened = policy
        .lock()
        .expect("the policy lock is not poisoned")
        .record_failure(millis(0), failure(FailureClass::Timeout));
    assert_eq!(opened, None);
    cancel_after_one_poll(run_async(&policy, &clock, "get", |_guard| PendingCall));
    let observed = policy
        .lock()
        .expect("the policy lock is not poisoned")
        .status();
    assert_eq!(
        observed,
        status(State::Closed, 0, 1, 0, 1),
        "a cancelled call records neither a success nor a failure"
    );
}

#[rstest]
#[case::connection(FailureClass::Connection, true, false)]
#[case::timeout(FailureClass::Timeout, true, true)]
#[case::busy_loading(FailureClass::BusyLoading, true, false)]
#[case::cluster_down(FailureClass::ClusterDown, true, false)]
#[case::os(FailureClass::Os, true, false)]
#[case::builtin_timeout(FailureClass::BuiltinTimeout, true, true)]
#[case::asynchronous_timeout(FailureClass::AsynchronousTimeout, true, true)]
#[case::command(FailureClass::Command, false, false)]
#[case::other(FailureClass::Other, false, false)]
#[case::circuit_open(FailureClass::CircuitOpen, false, false)]
fn failure_classes_classify(
    #[case] class: FailureClass,
    #[case] health: bool,
    #[case] timeout: bool,
) {
    assert_eq!(super::classify::is_health_failure(class), health);
    assert_eq!(is_timeout_failure(class, &[]), timeout);
    let expected = match (health, timeout) {
        (false, _) => None,
        (true, true) => Some(FailureKind::Timeout),
        (true, false) => Some(FailureKind::Connectivity),
    };
    assert_eq!(failure_kind(class, &[]), expected);
}

#[rstest]
#[case::connection(FailureClass::Connection, "connection")]
#[case::timeout(FailureClass::Timeout, "timeout")]
#[case::busy_loading(FailureClass::BusyLoading, "busy_loading")]
#[case::cluster_down(FailureClass::ClusterDown, "cluster_down")]
#[case::os(FailureClass::Os, "os")]
#[case::builtin_timeout(FailureClass::BuiltinTimeout, "builtin_timeout")]
#[case::asynchronous_timeout(FailureClass::AsynchronousTimeout, "asynchronous_timeout")]
#[case::command(FailureClass::Command, "command")]
#[case::other(FailureClass::Other, "other")]
#[case::circuit_open(FailureClass::CircuitOpen, "circuit_open")]
fn failure_class_names_match_the_probe_contract(#[case] class: FailureClass, #[case] name: &str) {
    let encoded = serde_json::to_string(&class).expect("a class encodes");
    assert_eq!(encoded, format!("\"{name}\""));
}

#[rstest]
#[case::closed(State::Closed, "closed")]
#[case::open(State::Open, "open")]
#[case::half_open(State::HalfOpen, "half_open")]
fn state_names_match_the_probe_contract(#[case] state: State, #[case] name: &str) {
    let encoded = serde_json::to_string(&state).expect("a state encodes");
    assert_eq!(encoded, format!("\"{name}\""));
}
