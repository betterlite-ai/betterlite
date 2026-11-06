use super::super::deadline;
use super::error::{
    ExecutionError, ExecutionTimeoutError, SandboxError, SandboxExit, sandbox_error,
};
use super::interrupt::cancel_task;
use std::future::Future;
use std::time::{Duration, Instant};
use tokio::task::JoinSet;
use tokio::time::timeout as deadline_timeout;
async fn contain<T, E, F>(pending: F, timeout: Duration) -> Result<T, ExecutionError<E>>
where
    F: Future<Output = Result<T, SandboxError<E>>>,
{
    match pending.await {
        Ok(value) => {
            if deadline::past_deadline() {
                return Err(ExecutionError::Timeout(ExecutionTimeoutError { timeout }));
            }
            Ok(value)
        }
        Err(error) => Err(sandbox_error(error, timeout)),
    }
}
pub async fn await_with_timeout<T, E, F>(
    pending: F,
    timeout: Duration,
    label: &str,
) -> Result<T, ExecutionError<E>>
where
    F: Future<Output = Result<T, SandboxError<E>>> + Send + 'static,
    E: Send + 'static,
    T: Send + 'static,
{
    let now = Instant::now();
    let deadline = now
        .checked_add(timeout)
        .ok_or(ExecutionError::TimeoutOutOfRange { timeout })?;
    let mut tasks = JoinSet::new();
    tasks.spawn(deadline::with_deadline(deadline, contain(pending, timeout)));
    match deadline_timeout(timeout, tasks.join_next()).await {
        Ok(Some(Ok(outcome))) => outcome,
        Ok(Some(Err(joined))) if joined.is_panic() => {
            let exit = SandboxExit::panic("the sandboxed task panicked".to_owned());
            Err(ExecutionError::Sandbox(SandboxError::Exit(exit)))
        }
        Ok(Some(Err(_cancelled))) => {
            Err(ExecutionError::Timeout(ExecutionTimeoutError { timeout }))
        }
        Ok(None) => Err(ExecutionError::Timeout(ExecutionTimeoutError { timeout })),
        Err(_elapsed) => {
            cancel_task(&mut tasks, label).await;
            Err(ExecutionError::Timeout(ExecutionTimeoutError { timeout }))
        }
    }
}
