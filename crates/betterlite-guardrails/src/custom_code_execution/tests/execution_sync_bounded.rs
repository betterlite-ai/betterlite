use std::thread;
use std::time::{Duration, Instant};

use super::super::deadline::budget_ok;
use super::super::execution::error::{ExecutionError, SandboxError};
use super::super::execution::sync::call_with_timeout;
use super::{LONG, SHORT};

#[test]
fn runs_the_work_on_a_worker_thread() {
    let caller = thread::current().id();
    let worker = call_with_timeout(
        move || Ok::<_, SandboxError<&'static str>>(thread::current().id()),
        LONG,
        "thread",
    )
    .unwrap();
    assert_ne!(caller, worker);
}

#[test]
fn returns_the_value_the_worker_produced() {
    let result: Result<String, ExecutionError<&'static str>> =
        call_with_timeout(|| Ok("allowed".to_owned()), LONG, "value");
    assert_eq!(result.unwrap(), "allowed");
}

#[test]
fn a_worker_stuck_outside_a_budget_check_still_times_out_on_schedule() {
    let start = Instant::now();
    let result: Result<(), ExecutionError<&'static str>> = call_with_timeout(
        || {
            thread::park_timeout(Duration::from_secs(5));
            Ok(())
        },
        SHORT,
        "stuck",
    );
    assert!(matches!(result, Err(ExecutionError::Timeout(_))));
    assert!(
        start.elapsed() < Duration::from_secs(2),
        "the caller waited {:?}",
        start.elapsed()
    );
}

#[test]
fn a_worker_that_checks_its_budget_ends_at_the_deadline() {
    let start = Instant::now();
    let result: Result<(), ExecutionError<&'static str>> = call_with_timeout(
        || loop {
            budget_ok().map_err(SandboxError::from)?;
        },
        SHORT,
        "budget",
    );
    assert!(matches!(result, Err(ExecutionError::Timeout(_))));
    assert!(start.elapsed() < Duration::from_secs(2));
}

#[test]
fn an_oversized_timeout_is_refused_before_a_worker_starts() {
    let result: Result<(), ExecutionError<&'static str>> =
        call_with_timeout(|| Ok(()), Duration::MAX, "huge");
    assert!(matches!(
        result,
        Err(ExecutionError::TimeoutOutOfRange { .. })
    ));
}
