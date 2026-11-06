use std::sync::mpsc::sync_channel;
use std::thread;
use std::time::{Duration, Instant};

use tokio::time::sleep;

use super::super::deadline::budget_ok;
use super::super::execution::error::{ExecutionError, SandboxError};
use super::super::execution::offloop::call_off_loop_with_timeout;
use super::{LONG, SHORT};

#[tokio::test]
async fn returns_the_value_the_worker_produced() {
    let result: Result<u8, ExecutionError<&'static str>> =
        call_off_loop_with_timeout(|| Ok(7), LONG, "value").await;
    assert_eq!(result.unwrap(), 7);
}

#[tokio::test]
async fn the_event_loop_runs_while_the_worker_waits() {
    let (sender, receiver) = sync_channel(1);
    let sender_task = async move {
        sleep(Duration::from_millis(20)).await;
        if sender.send(()).is_err() {
            tracing::debug!("the worker stopped waiting");
        }
    };
    let work = move || {
        receiver
            .recv_timeout(Duration::from_secs(2))
            .map_err(|_timed_out| SandboxError::Failed("the event loop never ran the sender"))
    };
    let (result, ()) = tokio::join!(
        call_off_loop_with_timeout(work, LONG, "waiting"),
        sender_task
    );
    result.unwrap();
}

#[tokio::test]
async fn a_worker_that_checks_its_budget_times_out() {
    let start = Instant::now();
    let result: Result<(), ExecutionError<&'static str>> = call_off_loop_with_timeout(
        || loop {
            budget_ok().map_err(SandboxError::from)?;
        },
        SHORT,
        "budget",
    )
    .await;
    assert!(matches!(result, Err(ExecutionError::Timeout(_))));
    assert!(start.elapsed() < Duration::from_secs(2));
}

#[tokio::test]
async fn a_worker_stuck_outside_a_budget_check_still_times_out_on_schedule() {
    let start = Instant::now();
    let result: Result<(), ExecutionError<&'static str>> = call_off_loop_with_timeout(
        || {
            thread::park_timeout(Duration::from_secs(5));
            Ok(())
        },
        SHORT,
        "stuck",
    )
    .await;
    assert!(matches!(result, Err(ExecutionError::Timeout(_))));
    assert!(start.elapsed() < Duration::from_secs(3));
}
