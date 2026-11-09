use std::future::Future;

use betterlite_lens::models::decode::DecodeError;
use betterlite_lens::models::execution::{ExecutionContent, Sample};
use betterlite_lens::models::messages::{Claim, JobResult, ModelRequest, ModelResult, Progress};
use thiserror::Error;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResultStatus {
    pub status: u16,
}

#[derive(Debug, Error)]
pub enum WorkerError {
    #[error("the worker timed out waiting for the proxy")]
    Timeout,
    #[error("the worker could not connect to the proxy")]
    Transport,
    #[error("the request to {path} answered with HTTP {status}")]
    Status { path: String, status: u16 },
    #[error("the worker could not read an analysis response: {0}")]
    Decode(String),
    #[error("the worker temporary storage failed: {0}")]
    Storage(String),
}

impl From<DecodeError> for WorkerError {
    fn from(error: DecodeError) -> Self {
        Self::Decode(error.to_string())
    }
}

impl From<serde_json::Error> for WorkerError {
    fn from(error: serde_json::Error) -> Self {
        Self::Decode(error.to_string())
    }
}

impl WorkerError {
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Timeout => "Timeout",
            Self::Transport => "Transport",
            Self::Status { .. } => "Status",
            Self::Decode(_text) => "Decode",
            Self::Storage(_text) => "Storage",
        }
    }
}

pub trait LensApi: Send + Sync + 'static {
    fn claim(&self) -> impl Future<Output = Result<Option<Claim>, WorkerError>> + Send;
    fn sample(&self, prefix: &str) -> impl Future<Output = Result<Sample, WorkerError>> + Send;
    fn content(
        &self,
        prefix: &str,
        execution_id: &str,
        cursor: &str,
        offset: i64,
    ) -> impl Future<Output = Result<ExecutionContent, WorkerError>> + Send;
    fn model(
        &self,
        prefix: &str,
        request: ModelRequest,
    ) -> impl Future<Output = Result<ModelResult, WorkerError>> + Send;
    fn progress(
        &self,
        prefix: &str,
        progress: Progress,
    ) -> impl Future<Output = Result<(), WorkerError>> + Send;
    fn heartbeat(&self, prefix: &str) -> impl Future<Output = Result<(), WorkerError>> + Send;
    fn result(
        &self,
        prefix: &str,
        result: &JobResult,
    ) -> impl Future<Output = Result<ResultStatus, WorkerError>> + Send;
}
