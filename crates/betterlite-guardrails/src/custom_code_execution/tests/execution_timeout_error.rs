use std::sync::Arc;
use std::time::{Duration, Instant};

use super::super::deadline::enter;
use super::super::execution::error::{ExecutionError, ExecutionInterrupted, SandboxError};
use super::super::execution::settle::settle;
use super::super::execution::sync::call_with_timeout;
use super::super::gate::InterruptGate;
use super::LONG;

#[test]
fn an_execution_interrupt_becomes_the_original_timeout() {
    let timeout = Duration::from_millis(250);
    let result: Result<(), ExecutionError<&'static str>> = call_with_timeout(
        || Err(SandboxError::Interrupted(ExecutionInterrupted)),
        timeout,
        "interrupt",
    );
    match result {
        Err(ExecutionError::Timeout(error)) => {
            assert_eq!(error.timeout, timeout);
            assert_eq!(error.to_string(), "exceeded the 250ms execution timeout");
        }
        other => panic!("expected the timeout, got {other:?}"),
    }
}

#[test]
fn a_result_that_arrives_after_the_deadline_is_a_timeout() {
    let budget = enter(
        Instant::now()
            .checked_sub(Duration::from_millis(1))
            .unwrap(),
        Arc::new(InterruptGate::new()),
    );
    let settled: Result<u8, SandboxError<&'static str>> = settle(|| Ok(7));
    drop(budget);
    assert!(matches!(settled, Err(SandboxError::Interrupted(_))));
}

#[test]
fn an_ordinary_failure_passes_through_the_boundary() {
    let result: Result<u8, ExecutionError<&'static str>> =
        call_with_timeout(|| Err(SandboxError::Failed("boom")), LONG, "failure");
    match result {
        Err(ExecutionError::Sandbox(SandboxError::Failed(message))) => {
            assert_eq!(message, "boom");
        }
        other => panic!("expected the sandbox failure, got {other:?}"),
    }
}

#[test]
fn a_successful_result_keeps_its_value() {
    let result: Result<u8, ExecutionError<&'static str>> =
        call_with_timeout(|| Ok(7), LONG, "value");
    assert!(matches!(result, Ok(7)));
}
