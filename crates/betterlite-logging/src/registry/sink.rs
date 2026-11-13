use std::fmt;
use std::sync::Arc;

use thiserror::Error;
use tokio::sync::mpsc;
use tokio::sync::mpsc::error::TrySendError;

use super::phases::CallbackPhase;
use crate::callback_logger::CallbackLogger;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CallbackEvent {
    pub phase: CallbackPhase,
}

#[derive(Debug, Error)]
pub enum SinkError {
    #[error("the callback channel is full")]
    Full,
    #[error("the callback channel is closed")]
    Closed,
}

#[derive(Clone)]
pub enum CallbackSink {
    Tracing,
    Channel(mpsc::Sender<CallbackEvent>),
    Logger(Arc<dyn CallbackLogger>),
}

// a trait object has no Debug, so the logger prints the one value every logger shares
impl fmt::Debug for CallbackSink {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Tracing => f.write_str("Tracing"),
            Self::Channel(_) => f.write_str("Channel"),
            Self::Logger(logger) => f
                .debug_tuple("Logger")
                .field(&logger.callback_name())
                .finish(),
        }
    }
}

impl CallbackSink {
    pub fn deliver(&self, event: &CallbackEvent) -> Result<(), SinkError> {
        match self {
            Self::Tracing => {
                tracing::info!(phase = %event.phase, "callback event");
                Ok(())
            }
            Self::Channel(sender) => sender
                .try_send(CallbackEvent { phase: event.phase })
                .map_err(SinkError::from),
            // a phase event carries no call; B250 and B251 hand loggers their events instead
            Self::Logger(_) => Ok(()),
        }
    }
}

impl From<TrySendError<CallbackEvent>> for SinkError {
    fn from(error: TrySendError<CallbackEvent>) -> Self {
        match error {
            TrySendError::Full(_) => Self::Full,
            TrySendError::Closed(_) => Self::Closed,
        }
    }
}
