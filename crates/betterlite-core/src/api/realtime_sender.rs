pub mod error;

mod backend;
mod client;
pub mod enforce;
mod setup;
mod tests;

use std::convert::Infallible;
use std::error::Error;
use std::future::Future;
use std::pin::Pin;

use crate::api::realtime_contracts::config::BaseRealtimeConfig;
use crate::api::realtime_guardrail_policy::RealtimeGuardrailCallback;
use crate::api::realtime_session_state::RealtimeStreaming;

// A sender call borrows the connection state, whose socket traits carry no Send bound, so it hands back a boxed local future instead of being an async fn
pub type SenderFuture<'sender, Output> = Pin<Box<dyn Future<Output = Output> + 'sender>>;

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
