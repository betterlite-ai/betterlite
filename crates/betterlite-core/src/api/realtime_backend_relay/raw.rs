use std::error::Error;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::api::realtime_beta_protocol::events::translate_event_to_beta;
use crate::api::realtime_contracts::config::BaseRealtimeConfig;
use crate::api::realtime_contracts::decode::decode_realtime_frame;
use crate::api::realtime_guardrail_policy::TRANSCRIPTION_EVENT_HOOKS;
use crate::api::realtime_guardrail_policy::disable_auto_response_message;
use crate::api::realtime_guardrail_policy::guardrails_run_for_hooks;
use crate::api::realtime_sender::error::RealtimeSenderError;
use crate::api::realtime_session_state::TRANSCRIPTION_COMPLETED;
use crate::api::realtime_session_state::error::SessionStateError;
use crate::api::realtime_session_state::records::LOGGING_FRAME_LIMITS;
use crate::api::realtime_session_state::store::SessionMessage;

mod store;

use self::store::store_and_forward_raw;
use super::RESPONSE_CREATE;
use super::RealtimeBackendRelay;
use super::error::RealtimeRelayError;
use super::event_type;
use super::transcript_and_item_id;
use super::transformed::guardrails::run_guardrails;
use super::transformed::send::Outbound;
use super::transformed::send::send;

// LiteLLM parses once and keeps only an object: non-JSON text, an array and a scalar all fall back to the raw frame
#[must_use]
pub fn parse_backend_event(raw_response: &str) -> Option<Json> {
    match decode_realtime_frame(raw_response.as_bytes(), LOGGING_FRAME_LIMITS) {
        Ok(event @ Json::Object(_)) => Some(event),
        Ok(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_))
        | Err(_) => None,
    }
}

// LiteLLM forwards a session.created only when a transcription guardrail is configured, then injects the auto-response disable
async fn handle_raw_backend_message<Config, CallbackError>(
    relay: &mut RealtimeBackendRelay<'_, Config, CallbackError>,
    event: &Json,
) -> Result<bool, RealtimeRelayError<Config::Error, CallbackError>>
where
    Config: BaseRealtimeConfig,
    CallbackError: Error + Send + 'static,
{
    let Json::Object(fields) = event else {
        return Ok(false);
    };
    let kind = event_type(event);
    relay.state.detect_transcription_session(fields);
    if kind == Some("session.created")
        && guardrails_run_for_hooks(
            &relay.callbacks,
            &relay.state.request_data,
            &TRANSCRIPTION_EVENT_HOOKS,
        )
        .map_err(RealtimeRelayError::Predicate)?
    {
        store_and_forward_raw(relay, fields).await?;
        let message = disable_auto_response_message(relay.state.backend_uses_beta_protocol.0)
            .map_err(|source| RealtimeRelayError::Send(RealtimeSenderError::Disable(source)))?;
        send(relay, Outbound::Backend(&message)).await?;
        return Ok(true);
    }
    if kind == Some(TRANSCRIPTION_COMPLETED) {
        raw_transcription_turn(relay, fields).await?;
        return Ok(true);
    }
    Ok(false)
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
    let (transcript, item_id) = transcript_and_item_id(fields);
    relay.state.collect_backend_input(fields);
    store_and_forward_raw(relay, fields).await?;
    if relay.state.is_transcription_session.0 {
        relay.state.capture_transcription_usage(fields);
        return Ok(());
    }
    let blocked = run_guardrails(relay, &transcript, item_id).await?;
    if blocked {
        return Ok(());
    }
    send(relay, Outbound::Backend(RESPONSE_CREATE)).await
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
        return relay.send_client_text(text).await;
    };
    if relay.sender().should_drop_event_from_client(&event) {
        return Ok(());
    }
    if handle_raw_backend_message(relay, &event).await? {
        return Ok(());
    }
    let normalized = relay.sender().normalize_event_for_ga_client(event);
    let Json::Object(fields) = &normalized else {
        return Err(RealtimeRelayError::State(SessionStateError::NotAnEvent));
    };
    relay
        .state
        .store_message(SessionMessage::Record(fields))
        .map_err(RealtimeRelayError::State)?;
    if !relay.state.client_wants_beta.0 {
        let encoded = serde_json::to_string(&normalized).map_err(RealtimeRelayError::Encode)?;
        return relay.send_client_text(&encoded).await;
    }
    let Some(translated) = translate_event_to_beta(normalized).map_err(RealtimeRelayError::Beta)?
    else {
        return Ok(());
    };
    let encoded = serde_json::to_string(&translated).map_err(RealtimeRelayError::Encode)?;
    relay.send_client_text(&encoded).await
}
