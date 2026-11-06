use std::sync::{Arc, Mutex, PoisonError};

use thiserror::Error;

#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
pub enum StreamFailure {
    #[error("the MCP response stream raised ValueError")]
    ValueError,
    #[error("the MCP response stream raised an HTTP error")]
    Http,
    #[error("the MCP response stream raised an OS error")]
    Io,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StreamMessage {
    Notification,
    Failure(StreamFailure),
}

#[derive(Clone, Debug, Default)]
pub struct MessageHandler {
    pub recorded: Arc<Mutex<Option<Arc<StreamFailure>>>>,
}

#[derive(Debug, Error, PartialEq, Eq)]
#[error("MCP response stream failed")]
pub struct StreamClosed;

impl MessageHandler {
    // The SDK closes pending requests when its message handler raises, so the first failure is kept and the call fails
    pub fn handle(&self, message: StreamMessage) -> Result<(), StreamClosed> {
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
