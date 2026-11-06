use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::thread;
use std::time::{Duration, Instant};

use tokio::task::JoinSet;
use tokio::time::timeout as deadline_timeout;

use super::super::deadline;
use super::super::gate::InterruptGate;
use super::error::{
    ExecutionError, ExecutionTimeoutError, SandboxError, SandboxExit, sandbox_error,
};
use super::interrupt::interrupt_worker;
use super::settle::settle;

// On cancellation a daemon thread keeps interrupting the worker and logs it stuck, as LiteLLM does.
struct InterruptOnCancel {
    gate: InterruptGate,
    label: String,
}

impl Drop for InterruptOnCancel {
    fn drop(&mut self) {
        if self.gate.interrupted.load(Ordering::SeqCst) {
            return;
        }
        let gate = self.gate.clone();
        let label = self.label.clone();
        let spawned = thread::Builder::new()
            .name(format!("guardrail-interrupt:{}", self.label))
            .spawn(move || interrupt_worker(&gate, || false, &label));
        if spawned.is_err() {
            tracing::error!("could not start the guardrail interrupt thread");
        }
    }
}

// Runs the sandboxed call on a worker thread and awaits it without blocking the event loop.
pub async fn call_off_loop_with_timeout<T, E, F>(
    work: F,
    timeout: Duration,
    label: &str,
) -> Result<T, ExecutionError<E>>
where
    F: FnOnce() -> Result<T, SandboxError<E>> + Send + 'static,
    E: Send + 'static,
    T: Send + 'static,
{
    let now = Instant::now();
    let deadline = now
        .checked_add(timeout)
        .ok_or(ExecutionError::TimeoutOutOfRange { timeout })?;
    let gate = InterruptGate::new();
    let _cancel = InterruptOnCancel {
        gate: gate.clone(),
        label: label.to_owned(),
    };
    let mut workers = JoinSet::new();
    let worker_gate = gate.clone();
    workers.spawn_blocking(move || {
        let _budget = deadline::enter(deadline, Arc::new(worker_gate.clone()));
        worker_gate.open();
        let outcome = settle(work);
        worker_gate.close();
        outcome
    });
    match deadline_timeout(timeout, workers.join_next()).await {
        Ok(Some(Ok(outcome))) => outcome.map_err(|error| sandbox_error(error, timeout)),
        Ok(Some(Err(joined))) if joined.is_panic() => {
            let exit = SandboxExit::panic("the guardrail worker panicked".to_owned());
            Err(ExecutionError::Sandbox(SandboxError::Exit(exit)))
        }
        Ok(Some(Err(_cancelled))) => {
            Err(ExecutionError::Timeout(ExecutionTimeoutError { timeout }))
        }
        Ok(None) => Err(ExecutionError::Timeout(ExecutionTimeoutError { timeout })),
        Err(_elapsed) => {
            let mut interrupts = JoinSet::new();
            let interrupt_gate = gate.clone();
            let name = label.to_owned();
            interrupts.spawn_blocking(move || interrupt_worker(&interrupt_gate, || false, &name));
            let _interrupted = interrupts.join_next().await;
            drop(workers);
            Err(ExecutionError::Timeout(ExecutionTimeoutError { timeout }))
        }
    }
}
