use std::path::Path;

use async_trait::async_trait;
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunResult {
    pub exit_code: i32,
    pub stdout: Option<String>,
    pub stderr: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionOptions {
    pub lang: &'static str,
    pub verbose: bool,
    pub image: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{message}")]
pub struct SdkFailure {
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SessionError {
    #[error("the llm-sandbox package is not installed")]
    SdkMissing,
    #[error("starting the sandbox session failed: {source}")]
    Start {
        #[source]
        source: SdkFailure,
    },
    #[error("the sandbox call {operation} failed: {source}")]
    Call {
        operation: &'static str,
        #[source]
        source: SdkFailure,
    },
}

impl From<String> for SdkFailure {
    fn from(message: String) -> Self {
        Self { message }
    }
}

#[async_trait]
pub trait SandboxSession: Send + Sync {
    async fn copy_to_runtime(&self, local: &Path, sandbox_path: &str) -> Result<(), SessionError>;

    async fn copy_from_runtime(&self, source: &str, destination: &Path)
    -> Result<(), SessionError>;

    async fn run(&self, code: &str) -> Result<RunResult, SessionError>;

    // the `with` block of the source closes the session before its result leaves
    async fn close(&self) -> Result<(), SessionError>;
}

#[async_trait]
pub trait SessionOpener: Send + Sync {
    async fn open(&self, options: &SessionOptions)
    -> Result<Box<dyn SandboxSession>, SessionError>;
}
