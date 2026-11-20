use std::error::Error;

use async_trait::async_trait;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::api::realtime_beta_protocol::events::translate_event_to_beta;
use crate::api::realtime_contracts::config::BaseRealtimeConfig;
use crate::api::realtime_contracts::decode::decode_realtime_frame;
use crate::api::realtime_guardrail_policy::TRANSCRIPTION_EVENT_HOOKS;
use crate::api::realtime_guardrail_policy::disable_auto_response_message;
use crate::api::realtime_guardrail_policy::guardrails_run_for_hooks;
use crate::api::realtime_sender::RealtimeSender;
use crate::api::realtime_sender::error::RealtimeSenderError;
use crate::api::realtime_session_state::TRANSCRIPTION_COMPLETED;
use crate::api::realtime_session_state::error::SessionStateError;
use crate::api::realtime_session_state::records::LOGGING_FRAME_LIMITS;
use crate::api::realtime_session_state::store::SessionMessage;

use super::RESPONSE_CREATE;
use super::RealtimeBackendRelay;
use super::error::RealtimeRelayError;
use super::event_type;
use super::send_client_text;
use super::transformed::guardrails::run_guardrails;
use super::transformed::send::send_to_backend;

// LiteLLM parses once and keeps only an object: non-JSON text, an array and a scalar all fall back to the raw frame
#[must_use]
pub fn parse_backend_event(raw_response: &str) -> Option<Json> {
    match decode_realtime_frame(raw_response.as_bytes(), LOGGING_FRAME_LIMITS) {
        Ok(event @ Json::Object(_)) => Some(event),
        Ok(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_))
        | Err(_) => None,
    }
}

#[async_trait]
pub(crate) trait HandleRawBackendMessage<Config, CallbackError>
where
    Config: BaseRealtimeConfig,
    CallbackError: Error + Send + 'static,
{
    async fn handle_raw_backend_message(
        &mut self,
        event: &Json,
    ) -> Result<bool, RealtimeRelayError<Config::Error, CallbackError>>;
}

#[async_trait]
impl<Config, CallbackError> HandleRawBackendMessage<Config, CallbackError>
    for RealtimeBackendRelay<'_, Config, CallbackError>
where
    Config: BaseRealtimeConfig,
    CallbackError: Error + Send + 'static,
{
    async fn handle_raw_backend_message(
        &mut self,
        event: &Json,
    ) -> Result<bool, RealtimeRelayError<Config::Error, CallbackError>> {
        let Json::Object(fields) = event else {
            return Ok(false);
        };
        let kind = event_type(event);
        self.state.detect_transcription_session(fields);
        if kind == Some("session.created")
            && guardrails_run_for_hooks(
                &self.callbacks,
                &self.state.request_data,
                &TRANSCRIPTION_EVENT_HOOKS,
            )
            .map_err(RealtimeRelayError::Predicate)?
        {
            raw_session_created(self, fields).await?;
            return Ok(true);
        }
        if kind == Some(TRANSCRIPTION_COMPLETED) {
            raw_transcription_turn(self, fields).await?;
            return Ok(true);
        }
        Ok(false)
    }
}

// The client sees session.created first, then the disable update goes out on every such event without the one-time flag
async fn raw_session_created<Config, CallbackError>(
    relay: &mut RealtimeBackendRelay<'_, Config, CallbackError>,
    fields: &IndexMap<String, Json>,
) -> Result<(), RealtimeRelayError<Config::Error, CallbackError>>
where
    Config: BaseRealtimeConfig,
    CallbackError: Error + Send + 'static,
{
    relay
        .state
        .store_message(SessionMessage::Record(fields))
        .map_err(RealtimeRelayError::State)?;
    let encoded = {
        let sender = RealtimeSender {
            state: &mut *relay.state,
            provider: relay.provider,
            guardrails: &relay.callbacks,
        };
        sender
            .event_to_client_json(Json::Object(fields.clone()))
            .map_err(RealtimeRelayError::Send)?
    };
    send_client_text(relay, &encoded).await?;
    let message = disable_auto_response_message(relay.state.backend_uses_beta_protocol.0)
        .map_err(|source| RealtimeRelayError::Send(RealtimeSenderError::Disable(source)))?;
    send_to_backend(relay, &message).await
}

// A transcription-only session bills the usage and never runs guardrails or asks for an assistant turn
async fn raw_transcription_turn<Config, CallbackError>(
    relay: &mut RealtimeBackendRelay<'_, Config, CallbackError>,
    fields: &IndexMap<String, Json>,
) -> Result<(), RealtimeRelayError<Config::Error, CallbackError>>
where
    Config: BaseRealtimeConfig,
    CallbackError: Error + Send + 'static,
{
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
    relay.state.collect_backend_input(fields);
    relay
        .state
        .store_message(SessionMessage::Record(fields))
        .map_err(RealtimeRelayError::State)?;
    let encoded = {
        let sender = RealtimeSender {
            state: &mut *relay.state,
            provider: relay.provider,
            guardrails: &relay.callbacks,
        };
        sender
            .event_to_client_json(Json::Object(fields.clone()))
            .map_err(RealtimeRelayError::Send)?
    };
    send_client_text(relay, &encoded).await?;
    if relay.state.is_transcription_session.0 {
        relay.state.capture_transcription_usage(fields);
        return Ok(());
    }
    let blocked = run_guardrails(relay, &transcript, item_id).await?;
    if blocked {
        return Ok(());
    }
    send_to_backend(relay, RESPONSE_CREATE).await
}

// Without a provider LiteLLM forwards the frame as-is, dropping only normalizer-rejected events, and beta clients get the translated form
pub(super) async fn relay_raw_frame<Config, CallbackError>(
    relay: &mut RealtimeBackendRelay<'_, Config, CallbackError>,
    text: &str,
) -> Result<(), RealtimeRelayError<Config::Error, CallbackError>>
where
    Config: BaseRealtimeConfig,
    CallbackError: Error + Send + 'static,
{
    let Some(event) = parse_backend_event(text) else {
        return send_client_text(relay, text).await;
    };
    let dropped = {
        let sender = RealtimeSender {
            state: &mut *relay.state,
            provider: relay.provider,
            guardrails: &relay.callbacks,
        };
        sender.should_drop_event_from_client(&event)
    };
    if dropped {
        return Ok(());
    }
    if relay.handle_raw_backend_message(&event).await? {
        return Ok(());
    }
    let normalized = {
        let sender = RealtimeSender {
            state: &mut *relay.state,
            provider: relay.provider,
            guardrails: &relay.callbacks,
        };
        sender.normalize_event_for_ga_client(event)
    };
    let Json::Object(fields) = &normalized else {
        return Err(RealtimeRelayError::State(SessionStateError::NotAnEvent));
    };
    relay
        .state
        .store_message(SessionMessage::Record(fields))
        .map_err(RealtimeRelayError::State)?;
    if !relay.state.client_wants_beta.0 {
        let encoded = serde_json::to_string(&normalized).map_err(RealtimeRelayError::Encode)?;
        return send_client_text(relay, &encoded).await;
    }
    let Some(translated) = translate_event_to_beta(normalized).map_err(RealtimeRelayError::Beta)?
    else {
        return Ok(());
    };
    let encoded = serde_json::to_string(&translated).map_err(RealtimeRelayError::Encode)?;
    send_client_text(relay, &encoded).await
}
