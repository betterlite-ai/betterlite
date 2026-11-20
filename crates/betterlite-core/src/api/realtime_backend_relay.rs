pub mod error;
pub mod raw;
pub mod relay;
pub mod source;
pub mod transformed;

mod tests;

use std::convert::Infallible;
use std::error::Error;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::api::realtime_contracts::config::BaseRealtimeConfig;
use crate::api::realtime_guardrail_policy::RealtimeGuardrailCallback;
use crate::api::realtime_guardrails::callback::RealtimeGuardrail;
use crate::api::realtime_sender::RealtimeSender;
use crate::api::realtime_session_state::RealtimeStreaming;
use crate::api::realtime_session_state::records;

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

// LiteLLM reads the type only from an object event and treats every other shape as untyped
pub(crate) fn event_type(event: &Json) -> Option<&str> {
    let Json::Object(fields) = event else {
        return None;
    };
    records::event_type(fields)
}

impl<Config, CallbackError> RealtimeBackendRelay<'_, Config, CallbackError>
where
    Config: BaseRealtimeConfig,
    CallbackError: Error + Send + 'static,
{
    // One sender around the same state, provider and callbacks for every step that writes to a socket
    pub(crate) fn sender(&mut self) -> RealtimeSender<'_, Config, CallbackError> {
        RealtimeSender {
            state: &mut *self.state,
            provider: self.provider,
            guardrails: &self.callbacks,
        }
    }

    // The raw path writes to the client socket directly, after the normalizer has already been asked about the event
    pub(crate) async fn send_client_text(
        &mut self,
        data: &str,
    ) -> Result<(), RealtimeRelayError<Config::Error, CallbackError>> {
        match self.state.client.as_mut() {
            Some(client) => client
                .send_text(data)
                .await
                .map_err(RealtimeRelayError::Client),
            None => Err(RealtimeRelayError::ClientMissing),
        }
    }
}

// LiteLLM reads transcript and item_id out of the completed event for the guardrails, defaulting the absent ones
pub(crate) fn transcript_and_item_id(fields: &IndexMap<String, Json>) -> (String, Option<&str>) {
    let transcript = match fields.get("transcript") {
        Some(Json::Text(text)) => text.clone(),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => String::new(),
    };
    let item_id = match fields.get("item_id") {
        Some(Json::Text(text)) => Some(text.as_str()),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => None,
    };
    (transcript, item_id)
}
