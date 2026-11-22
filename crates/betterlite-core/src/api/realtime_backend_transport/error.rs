use std::convert::Infallible;
use std::error::Error as StdError;

use thiserror::Error;

use crate::api::realtime_contracts::error::{RealtimeContractError, WebSocketError};
use crate::api::realtime_guardrail_policy::error::RealtimeGuardrailPolicyError;
use crate::api::realtime_session_state::error::SessionStateError;

// LiteLLM retries a timeout, an OSError and a websockets exception, but never a handshake rejection; the class decides both the retry and the close
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum DialError {
    #[error("{message}")]
    Timeout { message: String },
    #[error("{message}")]
    Os { message: String },
    #[error("{message}")]
    WebSocket { message: String },
    #[error("{message}")]
    InvalidStatus { status_code: u16, message: String },
    #[error("{message}")]
    InvalidStatusCode { status_code: u16, message: String },
    #[error("{message}")]
    Other { message: String },
}

impl DialError {
    // The three classes `_open_realtime_backend_ws` catches and retries; a deterministic rejection and any other failure skip the retry
    #[must_use]
    pub const fn is_retryable(&self) -> bool {
        matches!(
            self,
            Self::Timeout { .. } | Self::Os { .. } | Self::WebSocket { .. }
        )
    }
}

// LiteLLM tells a RuntimeError apart from every other close failure, because only a completed or already-closing socket is safe to ignore
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ClientCloseError {
    #[error("{message}")]
    Runtime { message: String },
    #[error("{message}")]
    Other { message: String },
}

// The two RuntimeError messages `async_realtime` swallows: the response already closed the socket, so its close call cannot run
#[must_use]
pub fn is_ignorable_close(error: &ClientCloseError) -> bool {
    match error {
        ClientCloseError::Runtime { message } => {
            message.contains("already completed") || message.contains("websocket.close")
        }
        ClientCloseError::Other { .. } => false,
    }
}

// Every step of async_realtime that can fail; the config, callback and relay errors come from the injected pieces
#[derive(Debug, Error)]
pub enum RealtimeConnectionError<ConfigError, CallbackError = Infallible, RelayError = Infallible>
where
    ConfigError: StdError + 'static,
    CallbackError: StdError + 'static,
    RelayError: StdError + 'static,
{
    #[error(transparent)]
    Config(ConfigError),
    #[error(transparent)]
    Dial(DialError),
    #[error(transparent)]
    Guardrail(RealtimeGuardrailPolicyError<CallbackError>),
    #[error(transparent)]
    Session(SessionStateError),
    #[error(transparent)]
    Contract(RealtimeContractError),
    #[error(transparent)]
    Relay(RelayError),
    #[error("the realtime backend websocket failed: {0}")]
    Backend(WebSocketError),
    #[error("the realtime client websocket failed: {0}")]
    Client(WebSocketError),
    #[error(transparent)]
    Close(ClientCloseError),
    #[error("Unexpected error while closing WebSocket: {close_error}")]
    UnexpectedClose {
        #[source]
        close_error: ClientCloseError,
    },
}
