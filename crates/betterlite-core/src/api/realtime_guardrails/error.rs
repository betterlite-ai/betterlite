use std::error::Error;

use thiserror::Error;

use crate::api::realtime_contracts::error::WebSocketError;
use crate::api::realtime_sender::error::RealtimeSenderError;

// Every step of a guardrail run that can fail; ConfigError comes from the provider transform and CallbackError from the guardrail
#[derive(Debug, Error)]
pub enum RealtimeGuardrailsError<ConfigError, CallbackError>
where
    ConfigError: Error + 'static,
    CallbackError: Error + 'static,
{
    #[error("the realtime guardrail predicate failed")]
    Predicate(#[source] CallbackError),
    #[error("the realtime guardrail raised an unexpected error")]
    Unexpected(#[source] CallbackError),
    #[error("the realtime guardrail backend send failed")]
    Send(#[source] RealtimeSenderError<ConfigError, CallbackError>),
    #[error("the realtime guardrail client notice failed")]
    Client(#[source] WebSocketError),
    #[error("the realtime guardrail backend close failed")]
    Close(#[source] WebSocketError),
    #[error("the realtime backend websocket is not connected")]
    BackendMissing,
    #[error("the realtime client websocket is not connected")]
    ClientMissing,
    #[error("the realtime guardrail frame cannot be written as JSON")]
    Encode(#[source] serde_json::Error),
}
