#![cfg(test)]

use std::future::Future;
use std::sync::Mutex;
use std::task::{Context, Poll, Waker};
use std::time::Duration;

use betterlite_redis::breaker_metrics::BreakerMetrics;

use crate::redis_failure_policy::classify::{Failure, FailureClass};
use crate::redis_failure_policy::run::{CallError, RedisCallError, run, run_async};
use crate::redis_failure_policy::{BreakerEvent, RedisFailurePolicy, Settings, State};

#[derive(Debug, Eq, PartialEq)]
enum TestError {
    Health,
    Other,
}

impl RedisCallError for TestError {
    fn failure(&self) -> Option<Failure<'_>> {
        match self {
            Self::Health => Some(Failure {
                class: FailureClass::Connection,
                causes: &[],
            }),
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

#[test]
fn a_successful_call_returns_the_value_and_records_success() {
    let clock = || Duration::from_millis(10);
    let policy = gate(3);
    let opened = policy
        .lock()
        .expect("the policy lock is not poisoned")
        .record_failure(Duration::from_millis(1), connection_failure());
    assert_eq!(opened, None);
    let outcome =
        run(&policy, &clock, "get", || Ok::<u32, TestError>(7)).expect("the breaker is closed");
    assert_eq!(outcome.value, 7);
    assert_eq!(outcome.warning, None);
    let status = policy
        .lock()
        .expect("the policy lock is not poisoned")
        .status();
    assert_eq!(status.failure_count, 0);
}

#[test]
fn a_health_failure_is_counted_and_reraised() {
    let clock = || Duration::from_millis(10);
    let policy = gate(1);
    let failure = run(&policy, &clock, "get", || {
        Err::<u32, TestError>(TestError::Health)
    })
    .expect_err("the call failed");
    match failure {
        CallError::Failed { error, warning } => {
            assert_eq!(error, TestError::Health);
            assert_eq!(
                warning,
                Some(BreakerEvent::Opened {
                    failure_count: 1,
                    hard_failure_count: 1,
                    recovery_timeout: Duration::from_secs(30),
                })
            );
        }
        CallError::Open(_) => panic!("the breaker was closed"),
    }
    let state = policy
        .lock()
        .expect("the policy lock is not poisoned")
        .status()
        .state;
    assert_eq!(state, State::Open);
}

#[test]
fn a_non_health_failure_is_reraised_without_counting() {
    let clock = || Duration::from_millis(10);
    let policy = gate(1);
    let failure = run(&policy, &clock, "get", || {
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
    let status = policy
        .lock()
        .expect("the policy lock is not poisoned")
        .status();
    assert_eq!(status.failure_count, 0);
}

#[test]
fn an_async_call_returns_its_value_and_records_success() {
    let clock = || Duration::from_millis(10);
    let policy = gate(3);
    let outcome = ready(run_async(&policy, &clock, "get", || async {
        Ok::<u32, TestError>(7)
    }))
    .expect("the breaker is closed");
    assert_eq!(outcome.value, 7);
    assert_eq!(outcome.warning, None);
    let state = policy
        .lock()
        .expect("the policy lock is not poisoned")
        .status()
        .state;
    assert_eq!(state, State::Closed);
}

#[test]
fn an_open_breaker_rejects_the_call_without_running_it() {
    let clock = || Duration::from_millis(10);
    let policy = gate(1);
    let opened = policy
        .lock()
        .expect("the policy lock is not poisoned")
        .record_failure(Duration::from_millis(1), connection_failure());
    assert!(opened.is_some());
    let mut ran = false;
    let failure = run(&policy, &clock, "get", || {
        ran = true;
        Ok::<u32, TestError>(7)
    })
    .expect_err("the breaker is open");
    assert!(!ran);
    assert!(matches!(failure, CallError::Open(_)));
}
