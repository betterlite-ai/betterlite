use std::convert::Infallible;
use std::error::Error;

use thiserror::Error;

use crate::api::realtime_beta_protocol::error::BetaProtocolError;
use crate::api::realtime_contracts::error::WebSocketError;
use crate::api::realtime_guardrail_policy::error::RealtimeGuardrailPolicyError;

// ConfigError comes from the provider config and CallbackError from a guardrail predicate; every variant names the step that raised it
#[derive(Debug, Error)]
pub enum RealtimeSenderError<ConfigError, CallbackError>
where
    ConfigError: Error + 'static,
    CallbackError: Error + 'static,
{
    #[error("the realtime provider transformation failed")]
    Transform {
        #[source]
        source: ConfigError,
    },
    #[error("the realtime guardrail predicate failed")]
    Predicate {
        #[source]
        source: CallbackError,
    },
    #[error("the realtime guardrail setup patch failed")]
    GuardrailSetup {
        #[source]
        source: RealtimeGuardrailPolicyError<CallbackError>,
    },
    #[error("the realtime auto-response disable message cannot be written as JSON")]
    Disable {
        #[source]
        source: RealtimeGuardrailPolicyError<Infallible>,
    },
    #[error("the realtime backend websocket is not connected")]
    BackendMissing,
    #[error("the realtime backend send failed")]
    Backend {
        #[source]
        source: WebSocketError,
    },
    #[error("the realtime client websocket is not connected")]
    ClientMissing,
    #[error("the realtime client send failed")]
    Client {
        #[source]
        source: WebSocketError,
    },
    #[error("the realtime event cannot be translated to the beta protocol")]
    Beta {
        #[source]
        source: BetaProtocolError,
    },
    #[error("the realtime session update is not a JSON object")]
    SessionUpdateNotObject,
    #[error("the realtime session update is nested past the depth the sender enforces")]
    SessionUpdateTooDeep,
    #[error("the realtime session update carries JSON this sender cannot decode")]
    SessionUpdateUnreadable,
    #[error("the realtime session update cannot be read")]
    SessionUpdateInvalid {
        #[source]
        source: serde_json::Error,
    },
    #[error("the realtime message cannot be written as JSON")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
}
