use std::future::Future;
use std::pin::Pin;

use async_trait::async_trait;
use betterlite_error::error::ErrorClass;
use thiserror::Error;

use super::logging::CallbackFailure;

pub type CloseResult = Result<(), ErrorClass>;

// LiteLLM catches Exception but lets a BaseException such as cancellation through without logging
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceFailure {
    Exception(ErrorClass),
    Cancellation,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
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
    #[error("the file content stream close task did not finish: {message}")]
    CleanupAborted { message: String },
}

pub trait SyncByteSource: Send {
    fn next_chunk(&mut self) -> Option<Result<Vec<u8>, SourceFailure>>;

    // None means the source has no close method, matching hasattr(stream, "close")
    fn close(&mut self) -> Option<CloseResult>;
}

// close() on an async source: None when it has none, Returned when it returned null,
// Awaitable when the caller must await what it returned
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
