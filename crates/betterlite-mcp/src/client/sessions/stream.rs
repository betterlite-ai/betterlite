use std::error::Error;
use std::sync::{Arc, Mutex, PoisonError};

use thiserror::Error;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StreamFailureKind {
    ValueError,
    Http,
    Io,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum StreamFailure<Source: Error + Send + Sync + 'static> {
    #[error("the MCP response stream raised ValueError")]
    ValueError(#[source] Arc<Source>),
    #[error("the MCP response stream raised an HTTP error")]
    Http(#[source] Arc<Source>),
    #[error("the MCP response stream raised an OS error")]
    Io(#[source] Arc<Source>),
}

impl<Source: Error + Send + Sync + 'static> StreamFailure<Source> {
    #[must_use]
    pub const fn kind(&self) -> StreamFailureKind {
        match self {
            Self::ValueError(_) => StreamFailureKind::ValueError,
            Self::Http(_) => StreamFailureKind::Http,
            Self::Io(_) => StreamFailureKind::Io,
        }
    }

    #[must_use]
    pub const fn source_error(&self) -> &Arc<Source> {
        match self {
            Self::ValueError(source) | Self::Http(source) | Self::Io(source) => source,
        }
    }
}

impl<Source: Error + Send + Sync + 'static> Clone for StreamFailure<Source> {
    fn clone(&self) -> Self {
        match self {
            Self::ValueError(source) => Self::ValueError(Arc::clone(source)),
            Self::Http(source) => Self::Http(Arc::clone(source)),
            Self::Io(source) => Self::Io(Arc::clone(source)),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StreamMessage<Source: Error + Send + Sync + 'static> {
    Notification,
    Failure(StreamFailure<Source>),
}

pub struct MessageHandler<Source: Error + Send + Sync + 'static> {
    pub recorded: Arc<Mutex<Option<Arc<StreamFailure<Source>>>>>,
}

impl<Source: Error + Send + Sync + 'static> Clone for MessageHandler<Source> {
    fn clone(&self) -> Self {
        Self {
            recorded: Arc::clone(&self.recorded),
        }
    }
}

impl<Source: Error + Send + Sync + 'static> Default for MessageHandler<Source> {
    fn default() -> Self {
        Self {
            recorded: Arc::new(Mutex::new(None)),
        }
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
#[error("MCP response stream failed")]
pub struct StreamClosed;

impl<Source: Error + Send + Sync + 'static> MessageHandler<Source> {
    // The SDK closes pending requests when its message handler raises, so the first failure is kept and the call fails
    pub fn handle(&self, message: StreamMessage<Source>) -> Result<(), StreamClosed> {
        let StreamMessage::Failure(failure) = message else {
            return Ok(());
        };
        let mut recorded = self.recorded.lock().unwrap_or_else(PoisonError::into_inner);
        if recorded.is_none() {
            *recorded = Some(Arc::new(failure));
        }
        drop(recorded);
        Err(StreamClosed)
    }
}
