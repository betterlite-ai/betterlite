use std::io;
use std::time::Duration;
use thiserror::Error;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("the sandboxed code used up its wall-clock budget")]
pub struct ExecutionInterrupted;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("exceeded the {timeout:?} execution timeout")]
pub struct ExecutionTimeoutError {
    pub timeout: Duration,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitKind {
    SystemExit,
    KeyboardInterrupt,
    BaseException,
    Panic,
}
impl ExitKind {
    const fn label(self) -> &'static str {
        match self {
            Self::SystemExit => "SystemExit",
            Self::KeyboardInterrupt => "KeyboardInterrupt",
            Self::BaseException => "BaseException",
            Self::Panic => "Panic",
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{}: {message}", kind.label())]
pub struct SandboxExit {
    pub kind: ExitKind,
    pub message: String,
}
impl SandboxExit {
    #[must_use]
    pub const fn system_exit(message: String) -> Self {
        Self {
            kind: ExitKind::SystemExit,
            message,
        }
    }
    #[must_use]
    pub const fn panic(message: String) -> Self {
        Self {
            kind: ExitKind::Panic,
            message,
        }
    }
}
#[derive(Debug, Error)]
pub enum SandboxError<E> {
    #[error(transparent)]
    Interrupted(ExecutionInterrupted),
    #[error(transparent)]
    Exit(SandboxExit),
    #[error(transparent)]
    Failed(E),
}
impl<E> From<ExecutionInterrupted> for SandboxError<E> {
    fn from(interrupted: ExecutionInterrupted) -> Self {
        Self::Interrupted(interrupted)
    }
}
#[derive(Debug, Error)]
pub enum ExecutionError<E> {
    #[error(transparent)]
    Timeout(ExecutionTimeoutError),
    #[error(transparent)]
    Sandbox(SandboxError<E>),
    #[error("could not start the guardrail worker: {source}")]
    Spawn {
        #[source]
        source: io::Error,
    },
    #[error("the execution timeout {timeout:?} is too large to schedule")]
    TimeoutOutOfRange { timeout: Duration },
}
impl<E> From<SandboxError<E>> for ExecutionError<E> {
    fn from(error: SandboxError<E>) -> Self {
        Self::Sandbox(error)
    }
}
#[must_use]
pub fn sandbox_error<E>(error: SandboxError<E>, timeout: Duration) -> ExecutionError<E> {
    match error {
        SandboxError::Interrupted(_) => ExecutionError::Timeout(ExecutionTimeoutError { timeout }),
        other @ (SandboxError::Exit(_) | SandboxError::Failed(_)) => ExecutionError::Sandbox(other),
    }
}
