#![cfg(test)]

use std::future::Future;
use std::sync::Mutex;
use std::task::{Context, Poll, Waker};
use std::time::Duration;

use betterlite_redis::breaker_metrics::BreakerMetrics;

use crate::redis_failure_policy::classify::{Failure, FailureClass};
use crate::redis_failure_policy::run::{
    CallError, CallGuard, CallOutcome, RedisCallError, run_async,
};
use crate::redis_failure_policy::{BreakerEvent, RedisFailurePolicy, Settings, State, Status};

#[derive(Debug, Eq, PartialEq)]
enum TestError {
    Health,
    Other,
}

impl RedisCallError for TestError {
    fn failure(&self) -> Option<Failure<'_>> {
        match self {
            Self::Health => Some(connection_failure()),
            Self::Other => None,
        }
    }
}

fn settings(threshold: u32) -> Settings {
    Settings {
        enabled: true,
        failure_threshold: threshold,
        recovery_timeout: Duration::from_secs(30),
        timeout_min_duration: Duration::from_millis(5_000),
        timeout_log_interval: Duration::from_secs(1),
    }
}

fn gate(threshold: u32) -> Mutex<RedisFailurePolicy> {
    Mutex::new(RedisFailurePolicy::new(
        settings(threshold),
        BreakerMetrics::new(None),
    ))
}

fn ready<F>(call: F) -> F::Output
where
    F: Future,
{
    let mut future = Box::pin(call);
    let waker = Waker::noop();
    let mut context = Context::from_waker(waker);
    match future.as_mut().poll(&mut context) {
        Poll::Ready(output) => output,
        Poll::Pending => panic!("the guarded call did not complete"),
    }
}

fn connection_failure() -> Failure<'static> {
    Failure {
        class: FailureClass::Connection,
        causes: &[],
    }
}

fn seed_failure(policy: &Mutex<RedisFailurePolicy>, at_millis: u64) -> Option<BreakerEvent> {
    policy
        .lock()
        .expect("the policy lock is not poisoned")
        .record_failure(Duration::from_millis(at_millis), connection_failure())
}

fn status(policy: &Mutex<RedisFailurePolicy>) -> Status {
    policy
        .lock()
        .expect("the policy lock is not poisoned")
        .status()
}

fn run_sync<'policy, T, E, F, C>(
    policy: &'policy Mutex<RedisFailurePolicy>,
    clock: &'policy C,
    operation: &str,
    call: F,
) -> Result<CallOutcome<T>, CallError<E>>
where
    F: FnOnce(&CallGuard<'policy, C>) -> Result<T, E>,
    E: RedisCallError,
    C: Fn() -> Duration + ?Sized,
{
    let guard = CallGuard::enter(policy, clock, operation).map_err(CallError::Open)?;
    guard.call(call)
}

fn swallowing_call<C>(
    policy: &Mutex<RedisFailurePolicy>,
    clock: &C,
    failure: Failure<'_>,
) -> Result<CallOutcome<Option<u32>>, CallError<TestError>>
where
    C: Fn() -> Duration + Sync,
{
    run_sync(policy, clock, "get", |guard| {
        let _event = guard.swallow(failure);
        Ok(None)
    })
}

#[test]
fn a_successful_call_returns_the_value_and_records_success() {
    let clock = || Duration::from_millis(10);
    let policy = gate(3);
    assert_eq!(seed_failure(&policy, 1), None);
    let outcome = run_sync(&policy, &clock, "get", |_guard| Ok::<u32, TestError>(7))
        .expect("the breaker is closed");
    assert_eq!(outcome.value, 7);
    assert_eq!(outcome.warning, None);
    assert_eq!(status(&policy).failure_count, 0);
}

#[test]
fn a_health_failure_is_counted_and_reraised() {
    let clock = || Duration::from_millis(10);
    let policy = gate(3);
    assert_eq!(seed_failure(&policy, 1), None);
    let failure = run_sync(&policy, &clock, "get", |_guard| {
        Err::<u32, TestError>(TestError::Health)
    })
    .expect_err("the call failed");
    match failure {
        CallError::Failed { error, warning } => {
            assert_eq!(error, TestError::Health);
            assert_eq!(warning, None);
        }
        CallError::Open(_) => panic!("the breaker was closed"),
    }
    let status = status(&policy);
    assert_eq!(status.failure_count, 2);
    assert_eq!(status.state, State::Closed);
}

#[test]
fn a_non_health_failure_is_reraised_without_counting() {
    let clock = || Duration::from_millis(10);
    let policy = gate(3);
    assert_eq!(seed_failure(&policy, 1), None);
    let failure = run_sync(&policy, &clock, "get", |_guard| {
        Err::<u32, TestError>(TestError::Other)
    })
    .expect_err("the call failed");
    match failure {
        CallError::Failed { error, warning } => {
            assert_eq!(error, TestError::Other);
            assert_eq!(warning, None);
        }
        CallError::Open(_) => panic!("the breaker was closed"),
    }
    assert_eq!(status(&policy).failure_count, 1);
}

#[test]
fn an_async_call_returns_its_value_and_records_success() {
    let clock = || Duration::from_millis(10);
    let policy = gate(3);
    assert_eq!(seed_failure(&policy, 1), None);
    let outcome = ready(run_async(&policy, &clock, "get", |_guard| async {
        Ok::<u32, TestError>(7)
    }))
    .expect("the breaker is closed");
    assert_eq!(outcome.value, 7);
    assert_eq!(outcome.warning, None);
    let status = status(&policy);
    assert_eq!(status.failure_count, 0);
    assert_eq!(status.state, State::Closed);
}

#[test]
fn an_open_breaker_rejects_the_call_without_running_it() {
    let clock = || Duration::from_millis(10);
    let policy = gate(1);
    assert!(seed_failure(&policy, 1).is_some());
    let mut ran = false;
    let failure = run_sync(&policy, &clock, "get", |_guard| {
        ran = true;
        Ok::<u32, TestError>(7)
    })
    .expect_err("the breaker is open");
    assert!(!ran);
    assert!(matches!(failure, CallError::Open(_)));
}

#[test]
fn a_call_that_swallows_a_health_failure_cannot_record_success() {
    let clock = || Duration::from_millis(10);
    let policy = gate(3);
    let first = swallowing_call(&policy, &clock, connection_failure());
    let second = swallowing_call(&policy, &clock, connection_failure());
    let third = swallowing_call(&policy, &clock, connection_failure());
    assert_eq!(first.expect("the breaker is closed").value, None);
    assert_eq!(second.expect("the breaker is closed").value, None);
    assert_eq!(third.expect("the breaker is closed").value, None);
    assert_eq!(
        status(&policy),
        Status {
            state: State::Open,
            generation: 1,
            failure_count: 3,
            hard_failure_count: 3,
            timeout_count: 0,
        },
        "three swallowed health failures open the breaker because no call records success"
    );
}

#[test]
fn a_call_that_swallows_a_non_health_failure_still_records_success() {
    let clock = || Duration::from_millis(10);
    let policy = gate(3);
    assert_eq!(seed_failure(&policy, 1), None);
    let outcome = run_sync(&policy, &clock, "get", |guard| {
        let _event = guard.swallow(Failure {
            class: FailureClass::Command,
            causes: &[],
        });
        Ok::<u32, TestError>(7)
    })
    .expect("the breaker is closed");
    assert_eq!(outcome.value, 7);
    assert_eq!(
        status(&policy).failure_count,
        0,
        "a non-health swallow does not block the success"
    );
}
