pub mod error;

mod backend;
mod client;
pub mod enforce;
mod methods;
mod tests;

use async_trait::async_trait;
use betterlite_models::decisions::Json;
use futures_util::future::BoxFuture;
use std::convert::Infallible;
use std::error::Error;

use self::error::RealtimeSenderError;

use crate::api::realtime_contracts::config::BaseRealtimeConfig;
use crate::api::realtime_guardrail_policy::RealtimeGuardrailCallback;
use crate::api::realtime_session_state::RealtimeStreaming;

pub type SenderFuture<'sender, Output, ConfigError, CallbackError> =
    BoxFuture<'sender, Result<Output, RealtimeSenderError<ConfigError, CallbackError>>>;

// LiteLLM's socket send boundary: B1006's connection state, B1004's provider config and B2455's guardrail callbacks
pub struct RealtimeSender<'sender, Config, CallbackError = Infallible>
where
    Config: BaseRealtimeConfig,
    CallbackError: Error + 'static,
{
    pub state: &'sender mut RealtimeStreaming,
    pub provider: Option<&'sender Config>,
    pub guardrails: &'sender [&'sender dyn RealtimeGuardrailCallback<Error = CallbackError>],
}

// The relays depend on this send boundary; B1125 drives it from axum's upgrade future.
#[async_trait]
pub trait RealtimeSend: Send {
    type Error: Error + 'static;

    async fn send_to_backend(&mut self, message: &str) -> Result<bool, Self::Error>;
    async fn send_event_to_client(
        &mut self,
        event: &Json,
        event_str: &str,
    ) -> Result<bool, Self::Error>;
    async fn maybe_disable_auto_response(&mut self) -> Result<(), Self::Error>;
}
