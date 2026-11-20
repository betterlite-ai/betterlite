pub mod error;
pub mod raw;
pub mod relay;
pub mod source;
pub mod transformed;

mod tests;

use std::convert::Infallible;
use std::error::Error;

use betterlite_models::decisions::Json;

use crate::api::realtime_contracts::config::BaseRealtimeConfig;
use crate::api::realtime_guardrail_policy::RealtimeGuardrailCallback;
use crate::api::realtime_guardrails::callback::RealtimeGuardrail;
use crate::api::realtime_session_state::RealtimeStreaming;

use error::RealtimeRelayError;
use source::RealtimeFailureDispatch;

// LiteLLM's backend-to-client relay: B1006's connection state, B1004's provider config, B2455/B1007's guardrails and B251's failure dispatch
pub struct RealtimeBackendRelay<'relay, Config, CallbackError = Infallible>
where
    Config: BaseRealtimeConfig,
    CallbackError: Error + Send + 'static,
{
    pub state: &'relay mut RealtimeStreaming,
    pub provider: Option<&'relay Config>,
    pub guardrails: &'relay [&'relay dyn RealtimeGuardrail<Error = CallbackError>],
    pub callbacks: Vec<&'relay dyn RealtimeGuardrailCallback<Error = CallbackError>>,
    pub failure_dispatch: &'relay dyn RealtimeFailureDispatch,
}

// LiteLLM's raw transcript branch sends response.create unless a guardrail block already produced the turn's frames
pub(crate) const RESPONSE_CREATE: &str = "{\"type\":\"response.create\"}";

// The sender reads the callback half of each guardrail while B1007's runner reads the whole trait, so the upcast happens once
pub fn relay_callbacks<'relay, CallbackError>(
    guardrails: &'relay [&'relay dyn RealtimeGuardrail<Error = CallbackError>],
) -> Vec<&'relay dyn RealtimeGuardrailCallback<Error = CallbackError>> {
    let mut callbacks = Vec::with_capacity(guardrails.len());
    for guardrail in guardrails {
        let callback: &dyn RealtimeGuardrailCallback<Error = CallbackError> = *guardrail;
        callbacks.push(callback);
    }
    callbacks
}

// The raw path writes to the client socket directly, after the normalizer has already been asked about the event
pub(crate) async fn send_client_text<Config, CallbackError>(
    relay: &mut RealtimeBackendRelay<'_, Config, CallbackError>,
    data: &str,
) -> Result<(), RealtimeRelayError<Config::Error, CallbackError>>
where
    Config: BaseRealtimeConfig,
    CallbackError: Error + Send + 'static,
{
    match relay.state.client.as_mut() {
        Some(client) => client
            .send_text(data)
            .await
            .map_err(RealtimeRelayError::Client),
        None => Err(RealtimeRelayError::ClientMissing),
    }
}

// LiteLLM reads the type only from an object event and treats every other shape as untyped
pub(crate) fn event_type(event: &Json) -> Option<&str> {
    match event {
        Json::Object(fields) => match fields.get("type") {
            Some(Json::Text(kind)) => Some(kind.as_str()),
            Some(
                Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_),
            )
            | None => None,
        },
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
    }
}
