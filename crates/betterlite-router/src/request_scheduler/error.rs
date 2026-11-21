use super::schedule::ScheduledFailure;
use crate::routing_value_store::StoreError;
use thiserror::Error;
// What a scheduler operation can refuse: a bound, a queue state, or the provider the scheduled call reached
#[derive(Debug, Error)]
pub enum SchedulerError {
    #[error("the scheduler queue for {model} holds the maximum of {limit} pending requests")]
    QueueFull { model: String, limit: usize },
    #[error("the request id {id} cannot be queued")]
    InvalidRequestId { id: String },
    #[error("the scheduler queue key is invalid")]
    Key {
        #[source]
        source: StoreError,
    },
    #[error("the value stored for the {model} scheduler queue is not a queue")]
    NotAQueue { model: String },
    #[error("the scheduler queue for {model} is empty")]
    EmptyQueue { model: String },
    #[error("the scheduler queue for {model} kept changing during the update")]
    Contended { model: String, attempts: u32 },
    #[error("request {request_id} timed out while waiting in the {model} scheduler queue")]
    TimedOut { model: String, request_id: String },
    #[error("request {request_id} was cancelled while waiting in the {model} scheduler queue")]
    Cancelled { model: String, request_id: String },
    #[error("the scheduled call failed")]
    Provider { failure: ScheduledFailure },
    #[error("this worker's scheduler store failed")]
    Local {
        #[source]
        source: StoreError,
    },
}
impl SchedulerError {
    // litellm.Timeout answers 408, the proxy writes a queue overflow as its 429, and a provider failure keeps its own status
    #[must_use]
    pub const fn status_code(&self) -> Option<u16> {
        match self {
            Self::QueueFull { .. } => Some(429),
            Self::TimedOut { .. } => Some(408),
            Self::InvalidRequestId { .. } => Some(400),
            Self::EmptyQueue { .. } => Some(500),
            Self::Provider { failure } => Some(failure.status),
            Self::Key { .. }
            | Self::NotAQueue { .. }
            | Self::Contended { .. }
            | Self::Cancelled { .. }
            | Self::Local { .. } => None,
        }
    }
    // litellm.Timeout carries no code, and a provider failure keeps the code its error reported
    #[must_use]
    pub fn code(&self) -> Option<&str> {
        match self {
            Self::Provider { failure } => failure.code.as_deref(),
            Self::QueueFull { .. }
            | Self::InvalidRequestId { .. }
            | Self::Key { .. }
            | Self::NotAQueue { .. }
            | Self::EmptyQueue { .. }
            | Self::Contended { .. }
            | Self::TimedOut { .. }
            | Self::Cancelled { .. }
            | Self::Local { .. } => None,
        }
    }
    // Router.schedule_acompletion's `setattr(e, "priority", priority)`
    #[must_use]
    pub const fn priority(&self) -> Option<u8> {
        match self {
            Self::Provider { failure } => Some(failure.priority),
            Self::QueueFull { .. }
            | Self::InvalidRequestId { .. }
            | Self::Key { .. }
            | Self::NotAQueue { .. }
            | Self::EmptyQueue { .. }
            | Self::Contended { .. }
            | Self::TimedOut { .. }
            | Self::Cancelled { .. }
            | Self::Local { .. } => None,
        }
    }
}
