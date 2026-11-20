use std::error::Error;

use thiserror::Error;

use crate::api::realtime_beta_protocol::error::BetaProtocolError;
use crate::api::realtime_contracts::error::WebSocketError;
use crate::api::realtime_guardrails::error::RealtimeGuardrailsError;
use crate::api::realtime_sender::error::RealtimeSenderError;
use crate::api::realtime_session_state::error::SessionStateError;
use crate::api::realtime_setup_buffer::RealtimeSetupBufferError;

use super::source::RelayReceiveError;

// Every step of the backend relay that can fail; ConfigError comes from the provider and CallbackError from a guardrail
#[derive(Debug, Error)]
pub enum RealtimeRelayError<ConfigError, CallbackError>
where
    ConfigError: Error + 'static,
    CallbackError: Error + 'static,
{
    #[error("the realtime backend websocket receive failed")]
    Receive(#[source] RelayReceiveError),
    #[error("the realtime provider transformation failed")]
    Transform(#[source] ConfigError),
    #[error("the realtime session state could not record the frame")]
    State(#[source] SessionStateError),
    #[error("the realtime guardrail predicate failed")]
    Predicate(#[source] CallbackError),
    #[error("the realtime sender failed")]
    Send(#[source] RealtimeSenderError<ConfigError, CallbackError>),
    #[error("the realtime guardrail run failed")]
    Guardrails(#[source] RealtimeGuardrailsError<ConfigError, CallbackError>),
    #[error("the realtime buffered client messages could not be replayed")]
    SetupBuffer(#[source] RealtimeSetupBufferError),
    #[error("the realtime client websocket is not connected")]
    ClientMissing,
    #[error("the realtime client send failed")]
    Client(#[source] WebSocketError),
    #[error("the realtime event cannot be written as JSON")]
    Encode(#[source] serde_json::Error),
    #[error("the realtime event cannot be translated to the beta protocol")]
    Beta(#[source] BetaProtocolError),
    #[error("the realtime relay was cancelled")]
    Cancelled,
}
