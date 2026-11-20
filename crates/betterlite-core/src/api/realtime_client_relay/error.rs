use std::error::Error;

use thiserror::Error;

use crate::api::realtime_beta_protocol::error::BetaProtocolError;
use crate::api::realtime_guardrails::error::RealtimeGuardrailsError;
use crate::api::realtime_sender::error::RealtimeSenderError;
use crate::api::realtime_setup_buffer::RealtimeSetupBufferError;

// Every step of the client loop that can fail; ConfigError comes from the provider and CallbackError from a guardrail
#[derive(Debug, Error)]
pub enum RealtimeClientRelayError<ConfigError, CallbackError>
where
    ConfigError: Error + 'static,
    CallbackError: Error + 'static,
{
    #[error("the realtime guardrail predicate failed")]
    Predicate(#[source] CallbackError),
    #[error("the realtime guardrail run failed")]
    Guardrails(#[source] RealtimeGuardrailsError<ConfigError, CallbackError>),
    #[error("the realtime sender failed")]
    Send(#[from] RealtimeSenderError<ConfigError, CallbackError>),
    #[error("the realtime client frame cannot be buffered")]
    SetupBuffer(#[from] RealtimeSetupBufferError),
    #[error("the realtime session update cannot be remapped for the GA protocol")]
    Remap(#[source] BetaProtocolError),
    #[error("the realtime guardrail input is not a list of text blocks")]
    GuardrailText,
    #[error("the realtime client frame cannot be written as JSON")]
    Encode(#[from] serde_json::Error),
}

impl<ConfigError, CallbackError> RealtimeClientRelayError<ConfigError, CallbackError>
where
    ConfigError: Error + 'static,
    CallbackError: Error + 'static,
{
    // LiteLLM returns BACKEND_CLOSED only for a closed backend socket; every other loop failure ends the client loop
    #[must_use]
    pub fn is_backend_closed(&self) -> bool {
        match self {
            Self::Send(error) => sender_closed_backend(error),
            Self::Guardrails(error) => guardrails_closed_backend(error),
            Self::Predicate(_)
            | Self::SetupBuffer(_)
            | Self::Remap(_)
            | Self::GuardrailText
            | Self::Encode(_) => false,
        }
    }
}

fn sender_closed_backend<ConfigError, CallbackError>(
    error: &RealtimeSenderError<ConfigError, CallbackError>,
) -> bool
where
    ConfigError: Error + 'static,
    CallbackError: Error + 'static,
{
    match error {
        RealtimeSenderError::Backend(_) => true,
        RealtimeSenderError::Transform(_)
        | RealtimeSenderError::Predicate(_)
        | RealtimeSenderError::GuardrailSetup(_)
        | RealtimeSenderError::Disable(_)
        | RealtimeSenderError::BackendMissing
        | RealtimeSenderError::ClientMissing
        | RealtimeSenderError::Client(_)
        | RealtimeSenderError::Beta(_)
        | RealtimeSenderError::SessionUpdateNotObject
        | RealtimeSenderError::SessionUpdateTooDeep
        | RealtimeSenderError::SessionUpdateUnreadable
        | RealtimeSenderError::SessionUpdateInvalid(_)
        | RealtimeSenderError::Encode(_) => false,
    }
}

fn guardrails_closed_backend<ConfigError, CallbackError>(
    error: &RealtimeGuardrailsError<ConfigError, CallbackError>,
) -> bool
where
    ConfigError: Error + 'static,
    CallbackError: Error + 'static,
{
    match error {
        RealtimeGuardrailsError::Send(error) => sender_closed_backend(error),
        RealtimeGuardrailsError::Close(_) => true,
        RealtimeGuardrailsError::Predicate(_)
        | RealtimeGuardrailsError::Unexpected(_)
        | RealtimeGuardrailsError::Client(_)
        | RealtimeGuardrailsError::BackendMissing
        | RealtimeGuardrailsError::ClientMissing
        | RealtimeGuardrailsError::Encode(_) => false,
    }
}
