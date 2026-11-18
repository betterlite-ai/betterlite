pub mod error;

mod backend;
mod client;
pub mod enforce;
mod methods;
mod tests;

use async_trait::async_trait;
use betterlite_models::decisions::Json;
use std::convert::Infallible;
use std::error::Error;

use self::error::RealtimeSenderError;

use crate::api::realtime_contracts::config::BaseRealtimeConfig;
use crate::api::realtime_guardrail_policy::RealtimeGuardrailCallback;
use crate::api::realtime_session_state::RealtimeStreaming;

// Private helpers borrowing the connection state share the same local future and error type as the send interface.
#[rustfmt::skip]
#[expect(clippy::disallowed_types, reason = "the realtime relay state holds connection trait objects that are not Send yet")]
pub type SenderFuture<'sender, Output, ConfigError, CallbackError> =
    futures_util::future::LocalBoxFuture<'sender, Result<Output, RealtimeSenderError<ConfigError, CallbackError>>>;

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

// The relays depend on this send boundary; its futures stay local because connection callbacks are not Send.
#[async_trait(?Send)]
pub trait RealtimeSend {
    type Error: Error + 'static;

    async fn send_to_backend(&mut self, message: &str) -> Result<bool, Self::Error>;
    async fn send_event_to_client(
        &mut self,
        event: &Json,
        event_str: &str,
    ) -> Result<bool, Self::Error>;
    async fn maybe_disable_auto_response(&mut self) -> Result<(), Self::Error>;
}
