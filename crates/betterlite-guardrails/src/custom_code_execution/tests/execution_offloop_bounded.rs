use std::sync::mpsc::sync_channel;
use std::thread;
use std::time::{Duration, Instant};

use tokio::time::sleep;
use tokio::time::timeout as deadline_timeout;

use super::super::deadline::budget_ok;
use super::super::execution::error::{ExecutionError, SandboxError};
use super::super::execution::interrupt::{INTERRUPT_GRACE, INTERRUPT_POLL};
use super::super::execution::offloop::call_off_loop_with_timeout;
use super::{LONG, SHORT, budget_worker, captured_logs};

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

#[test]
fn the_offloop_interrupt_grace_and_poll_match_the_synchronous_path() {
    assert_eq!(INTERRUPT_GRACE, Duration::from_secs(1));
    assert_eq!(INTERRUPT_POLL, Duration::from_millis(50));
}

#[tokio::test]
async fn a_stuck_worker_spends_the_grace_period_and_is_logged() {
    let _logs = captured_logs();
    let start = Instant::now();
    let result: Result<(), ExecutionError<&'static str>> = call_off_loop_with_timeout(
        || {
            thread::park_timeout(Duration::from_secs(5));
            Ok(())
        },
        SHORT,
        "stuck-offloop",
    )
    .await;
    assert!(matches!(result, Err(ExecutionError::Timeout(_))));
    assert!(
        start.elapsed() >= INTERRUPT_GRACE,
        "the caller returned after {:?}",
        start.elapsed()
    );
    assert!(
        captured_logs().contains("guardrail-code:stuck-offloop is still running"),
        "the stuck worker was not logged"
    );
}

#[tokio::test]
async fn a_worker_sees_its_budget_expire_at_the_deadline() {
    let (sender, receiver) = sync_channel(1);
    let result: Result<(), ExecutionError<&'static str>> = call_off_loop_with_timeout(
        move || budget_worker(&sender),
        SHORT,
        "budget-signal-offloop",
    )
    .await;
    assert!(matches!(result, Err(ExecutionError::Timeout(_))));
    receiver.recv_timeout(Duration::from_secs(2)).unwrap();
}

#[tokio::test]
async fn dropping_the_future_interrupts_the_worker() {
    let (sender, receiver) = sync_channel(1);
    let work = move || budget_worker(&sender);
    let dropped = deadline_timeout(Duration::from_millis(30), async {
        let outcome: Result<(), ExecutionError<&'static str>> =
            call_off_loop_with_timeout(work, LONG, "cancel").await;
        outcome
    })
    .await;
    dropped.unwrap_err();
    receiver.recv_timeout(Duration::from_secs(2)).unwrap();
}
