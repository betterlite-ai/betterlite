use std::sync::mpsc::sync_channel;
use std::thread;
use std::time::{Duration, Instant};

use super::super::deadline::budget_ok;
use super::super::execution::error::{ExecutionError, SandboxError};
use super::super::execution::interrupt::{INTERRUPT_GRACE, INTERRUPT_POLL};
use super::super::execution::sync::call_with_timeout;
use super::{LONG, SHORT, budget_worker, captured_logs};

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

#[test]
fn the_interrupt_grace_and_poll_are_the_ones_the_source_preserves() {
    assert_eq!(INTERRUPT_GRACE, Duration::from_secs(1));
    assert_eq!(INTERRUPT_POLL, Duration::from_millis(50));
}

#[test]
fn a_stuck_worker_spends_the_grace_period_and_is_logged() {
    let _logs = captured_logs();
    let start = Instant::now();
    let result: Result<(), ExecutionError<&'static str>> = call_with_timeout(
        || {
            thread::park_timeout(Duration::from_secs(5));
            Ok(())
        },
        SHORT,
        "stuck-grace",
    );
    assert!(matches!(result, Err(ExecutionError::Timeout(_))));
    assert!(
        start.elapsed() >= INTERRUPT_GRACE,
        "the caller returned after {:?}",
        start.elapsed()
    );
    assert!(
        captured_logs().contains("guardrail-code:stuck-grace is still running"),
        "the stuck worker was not logged"
    );
}

#[test]
fn a_worker_sees_its_budget_expire_at_the_deadline() {
    let (sender, receiver) = sync_channel(1);
    let result: Result<(), ExecutionError<&'static str>> =
        call_with_timeout(move || budget_worker(&sender), SHORT, "budget-signal");
    assert!(matches!(result, Err(ExecutionError::Timeout(_))));
    receiver.recv_timeout(Duration::from_secs(2)).unwrap();
}
