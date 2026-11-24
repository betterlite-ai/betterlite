use std::future::Future;
use std::pin::Pin;

use async_trait::async_trait;
use betterlite_error::error::ErrorClass;
use thiserror::Error;
use tokio::task::JoinError;

use super::logging::CallbackFailure;

pub type CloseResult = Result<(), ErrorClass>;

// LiteLLM catches Exception but lets a BaseException such as cancellation through without logging
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceFailure {
    Exception(ErrorClass),
    Cancellation,
}

#[derive(Debug, Error)]
pub enum StreamError {
    #[error("File content stream does not support sync iteration")]
    UnsupportedSync,
    #[error("File content stream does not support async iteration")]
    UnsupportedAsync,
    #[error("the file content stream failed")]
    Source { failure: ErrorClass },
    #[error("the file content stream was cancelled")]
    Cancelled,
    #[error("a file content stream callback failed: {failure}")]
    Callback { failure: CallbackFailure },
    #[error("the file content stream could not be closed")]
    Cleanup { failure: ErrorClass },
    #[error("the file content stream close task did not finish")]
    CleanupAborted {
        #[source]
        source: JoinError,
    },
}

impl StreamError {
    // The source error wins unless a failure callback raised first
    pub(super) fn from_failure(error: ErrorClass, logged: Result<(), Self>) -> Self {
        match logged {
            Err(failure) => failure,
            Ok(()) => Self::Source { failure: error },
        }
    }
}

impl From<CallbackFailure> for StreamError {
    fn from(failure: CallbackFailure) -> Self {
        Self::Callback { failure }
    }
}

pub trait SyncByteSource: Send {
    fn next_chunk(&mut self) -> Option<Result<Vec<u8>, SourceFailure>>;

    // None means the source has no close method, matching hasattr(stream, "close")
    fn close(&mut self) -> Option<CloseResult>;
}

// close() on an async source: Absent without one, Returned for null, Awaitable for a result
pub enum CloseCall<'source> {
    Absent,
    Returned,
    Awaitable(Pin<Box<dyn Future<Output = CloseResult> + Send + 'source>>),
}

#[async_trait]
pub trait AsyncByteSource: Send {
    async fn next_chunk(&mut self) -> Option<Result<Vec<u8>, SourceFailure>>;

    // None means the source has no aclose method, matching hasattr(stream, "aclose")
    async fn aclose(&mut self) -> Option<CloseResult>;

    fn close(&mut self) -> CloseCall<'_>;
}

pub enum StreamSource {
    Sync(Box<dyn SyncByteSource>),
    Async(Box<dyn AsyncByteSource>),
}

impl StreamSource {
    pub(super) fn sync_source(&mut self) -> Result<&mut dyn SyncByteSource, StreamError> {
        match self {
            Self::Sync(source) => Ok(&mut **source),
            Self::Async(_) => Err(StreamError::UnsupportedSync),
        }
    }

    pub(super) fn async_source(&mut self) -> Result<&mut dyn AsyncByteSource, StreamError> {
        match self {
            Self::Async(source) => Ok(&mut **source),
            Self::Sync(_) => Err(StreamError::UnsupportedAsync),
        }
    }
}

// The synchronous iter(()) LiteLLM swaps in once it takes the real stream for cleanup
pub struct EmptySource;

impl SyncByteSource for EmptySource {
    fn next_chunk(&mut self) -> Option<Result<Vec<u8>, SourceFailure>> {
        None
    }

    fn close(&mut self) -> Option<CloseResult> {
        None
    }
}
