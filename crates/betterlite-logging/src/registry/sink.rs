use thiserror::Error;
use tokio::sync::mpsc;
use tokio::sync::mpsc::error::TrySendError;

use super::phases::CallbackPhase;

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

// Both implementations are built by the server and passed in; neither carries a configurable name
#[derive(Debug, Clone)]
pub enum CallbackSink {
    Tracing,
    Channel(mpsc::Sender<CallbackEvent>),
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
