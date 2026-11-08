use super::super::deadline;
use super::super::gate::InterruptGate;
use super::error::{
    ExecutionError, ExecutionTimeoutError, SandboxError, SandboxExit, sandbox_error,
};
use super::interrupt::interrupt_worker;
use super::settle::settle;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, sync_channel};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
pub fn call_with_timeout<T, E, F>(
    work: F,
    timeout: Duration,
    label: &str,
) -> Result<T, ExecutionError<E>>
where
    F: FnOnce() -> Result<T, SandboxError<E>> + Send + 'static,
    E: Send + 'static,
    T: Send + 'static,
{
    let deadline = Instant::now()
        .checked_add(timeout)
        .ok_or(ExecutionError::TimeoutOutOfRange { timeout })?;
    let gate = InterruptGate::new();
    let (sender, receiver) = sync_channel(1);
    let handle = spawn_worker(work, deadline, Arc::new(gate.clone()), sender, label)?;
    wait_for_worker(&receiver, handle, timeout, &gate, label)
}
fn spawn_worker<T, E, F>(
    work: F,
    deadline: Instant,
    gate: Arc<InterruptGate>,
    sender: SyncSender<Result<T, SandboxError<E>>>,
    label: &str,
) -> Result<JoinHandle<()>, ExecutionError<E>>
where
    F: FnOnce() -> Result<T, SandboxError<E>> + Send + 'static,
    E: Send + 'static,
    T: Send + 'static,
{
    thread::Builder::new()
        .name(format!("guardrail-code:{label}"))
        .spawn(move || {
            let _budget = deadline::enter(deadline, Arc::clone(&gate));
            gate.open();
            let outcome = settle(work);
            gate.close();
            if sender.send(outcome).is_err() {
                tracing::debug!("the caller already returned its timeout");
            }
        })
        .map_err(|source| ExecutionError::Spawn { source })
}
fn wait_for_worker<T, E>(
    receiver: &Receiver<Result<T, SandboxError<E>>>,
    handle: JoinHandle<()>,
    timeout: Duration,
    gate: &InterruptGate,
    label: &str,
) -> Result<T, ExecutionError<E>> {
    match receiver.recv_timeout(timeout) {
        Ok(Ok(value)) => {
            reap(handle);
            Ok(value)
        }
        Ok(Err(error)) => {
            reap(handle);
            Err(sandbox_error(error, timeout))
        }
        Err(RecvTimeoutError::Timeout) => {
            interrupt_worker(gate, || handle.is_finished(), label);
            if handle.is_finished() {
                reap(handle);
            }
            Err(ExecutionError::Timeout(ExecutionTimeoutError { timeout }))
        }
        Err(RecvTimeoutError::Disconnected) => {
            let message = match handle.join() {
                Err(_panic) => "the guardrail worker panicked",
                Ok(()) => "the guardrail worker stopped without a result",
            };
            Err(ExecutionError::Sandbox(SandboxError::Exit(
                SandboxExit::panic(message.to_owned()),
            )))
        }
    }
}
fn reap(handle: JoinHandle<()>) {
    if handle.join().is_err() {
        tracing::error!("the guardrail worker panicked after delivering its result");
    }
}
