use super::super::deadline;
use super::error::{ExecutionInterrupted, SandboxError};
pub fn settle<T, E, F>(work: F) -> Result<T, SandboxError<E>>
where
    F: FnOnce() -> Result<T, SandboxError<E>>,
{
    let outcome = work();
    match outcome {
        Ok(_) if deadline::past_deadline() => Err(SandboxError::Interrupted(ExecutionInterrupted)),
        other => other,
    }
}
