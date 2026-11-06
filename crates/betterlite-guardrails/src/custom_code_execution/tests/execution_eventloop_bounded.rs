use std::future::{pending, ready};
use std::thread;
use std::time::{Duration, Instant};

use tokio::time::sleep;

use super::super::execution::error::{
    ExecutionError, ExecutionInterrupted, ExitKind, SandboxError, SandboxExit,
};
use super::super::execution::eventloop::await_with_timeout;
use super::{LONG, SHORT};

#[tokio::test]
async fn returns_the_pending_result() {
    let result: Result<u8, ExecutionError<&'static str>> =
        await_with_timeout(ready(Ok(7)), LONG, "value").await;
    assert_eq!(result.unwrap(), 7);
}

#[tokio::test]
async fn an_execution_interrupt_inside_the_task_becomes_a_timeout() {
    let result: Result<u8, ExecutionError<&'static str>> = await_with_timeout(
        ready(Err(SandboxError::Interrupted(ExecutionInterrupted))),
        SHORT,
        "interrupt",
    )
    .await;
    match result {
        Err(ExecutionError::Timeout(error)) => assert_eq!(error.timeout, SHORT),
        other => panic!("expected the timeout, got {other:?}"),
    }
}

#[tokio::test]
async fn a_pending_future_is_given_up_at_the_deadline() {
    let start = Instant::now();
    let result: Result<u8, ExecutionError<&'static str>> =
        await_with_timeout(pending(), SHORT, "pending").await;
    assert!(matches!(result, Err(ExecutionError::Timeout(_))));
    assert!(
        start.elapsed() < Duration::from_secs(2),
        "waited {:?}",
        start.elapsed()
    );
}

#[tokio::test]
async fn a_result_that_arrives_after_the_deadline_is_a_timeout() {
    let late = async {
        sleep(Duration::from_millis(80)).await;
        Ok::<u8, SandboxError<&'static str>>(1)
    };
    let result = await_with_timeout(late, Duration::from_millis(10), "late").await;
    assert!(matches!(result, Err(ExecutionError::Timeout(_))));
}

#[tokio::test]
async fn a_panicking_task_becomes_a_sandbox_exit() {
    let result: Result<(), ExecutionError<&'static str>> =
        await_with_timeout(async { panic!("task exploded") }, LONG, "panic").await;
    match result {
        Err(ExecutionError::Sandbox(SandboxError::Exit(exit))) => {
            assert_eq!(exit.kind, ExitKind::Panic);
            assert_eq!(
                exit,
                SandboxExit::panic("the sandboxed task panicked".to_owned())
            );
        }
        other => panic!("expected a panic exit, got {other:?}"),
    }
}

#[tokio::test]
async fn an_ordinary_failure_crosses_the_task_boundary_unchanged() {
    let result: Result<u8, ExecutionError<&'static str>> =
        await_with_timeout(ready(Err(SandboxError::Failed("bad"))), LONG, "failure").await;
    assert!(matches!(
        result,
        Err(ExecutionError::Sandbox(SandboxError::Failed("bad")))
    ));
}

#[tokio::test]
async fn a_success_that_lands_after_the_deadline_is_a_timeout() {
    let late = async {
        thread::park_timeout(Duration::from_millis(60));
        Ok::<u8, SandboxError<&'static str>>(1)
    };
    let result = await_with_timeout(late, Duration::from_millis(10), "late-ok").await;
    assert!(matches!(result, Err(ExecutionError::Timeout(_))));
}

#[tokio::test]
async fn a_failure_that_lands_after_the_deadline_stays_a_failure() {
    let late = async {
        thread::park_timeout(Duration::from_millis(60));
        Err::<u8, _>(SandboxError::Failed("late failure"))
    };
    let result = await_with_timeout(late, Duration::from_millis(10), "late-fail").await;
    assert!(matches!(
        result,
        Err(ExecutionError::Sandbox(SandboxError::Failed(
            "late failure"
        )))
    ));
}
