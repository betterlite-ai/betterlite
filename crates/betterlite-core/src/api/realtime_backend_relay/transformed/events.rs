use std::error::Error;

use betterlite_models::decisions::Json;

use crate::api::realtime_contracts::config::BaseRealtimeConfig;
use crate::api::realtime_guardrail_policy::TRANSCRIPTION_EVENT_HOOKS;
use crate::api::realtime_guardrail_policy::guardrails_run_for_hooks;
use crate::api::realtime_session_state::TRANSCRIPTION_COMPLETED;
use crate::api::realtime_session_state::store::SessionMessage;

use super::super::RESPONSE_CREATE;
use super::super::RealtimeBackendRelay;
use super::super::error::RealtimeRelayError;
use super::super::event_type;
use super::super::transcript_and_item_id;
use super::guardrails::run_guardrails;
use super::send::Outbound;
use super::send::send;
use super::setup::complete_deferred_setup;

// One transformed event: the session-ready flush and guardrail turns run before the plain store-and-forward
pub(super) async fn provider_event<Config, CallbackError>(
    relay: &mut RealtimeBackendRelay<'_, Config, CallbackError>,
    mut event: Json,
) -> Result<(), RealtimeRelayError<Config::Error, CallbackError>>
where
    Config: BaseRealtimeConfig,
    CallbackError: Error + Send + 'static,
{
    if relay.sender().should_drop_event_from_client(&event) {
        return Ok(());
    }
    let session_created = event_type(&event) == Some("session.created");
    if session_created {
        complete_deferred_setup(relay).await?;
        if relay.state.setup.session_created_sent_to_client.0 {
            event = retype_as_session_updated(event);
        } else {
            relay.state.setup.session_created_sent_to_client.0 = true;
        }
    }
    let event_str = serde_json::to_string(&event).map_err(RealtimeRelayError::Encode)?;
    if session_created
        && guardrails_run_for_hooks(
            &relay.callbacks,
            &relay.state.request_data,
            &TRANSCRIPTION_EVENT_HOOKS,
        )
        .map_err(RealtimeRelayError::Predicate)?
    {
        relay
            .state
            .store_message(SessionMessage::Text(&event_str))
            .map_err(RealtimeRelayError::State)?;
        send(relay, Outbound::Event(&event, &event_str)).await?;
        send(relay, Outbound::DisableAutoResponse).await?;
        return Ok(());
    }
    if event_type(&event) == Some(TRANSCRIPTION_COMPLETED) {
        emit_transcription_turn(relay, &event, &event_str).await?;
        return Ok(());
    }
    relay
        .state
        .store_message(SessionMessage::Text(&event_str))
        .map_err(RealtimeRelayError::State)?;
    send(relay, Outbound::Event(&event, &event_str)).await
}

// A completed transcript is logged and forwarded before the guardrails see it, and only a clean conversational turn asks for a reply
async fn emit_transcription_turn<Config, CallbackError>(
    relay: &mut RealtimeBackendRelay<'_, Config, CallbackError>,
    event: &Json,
    event_str: &str,
) -> Result<(), RealtimeRelayError<Config::Error, CallbackError>>
where
    Config: BaseRealtimeConfig,
    CallbackError: Error + Send + 'static,
{
    let Json::Object(fields) = event else {
        return Ok(());
    };
    let (transcript, item_id) = transcript_and_item_id(fields);
    relay.state.collect_backend_input(fields);
    relay
        .state
        .store_message(SessionMessage::Text(event_str))
        .map_err(RealtimeRelayError::State)?;
    relay.state.capture_transcription_usage(fields);
    send(relay, Outbound::Event(event, event_str)).await?;
    let blocked = run_guardrails(relay, &transcript, item_id).await?;
    if !blocked && !relay.state.is_transcription_session.0 {
        send(relay, Outbound::Backend(RESPONSE_CREATE)).await?;
    }
    Ok(())
}

// LiteLLM re-emits the provider's real session.created as session.updated once a synthetic one reached the client
fn retype_as_session_updated(mut event: Json) -> Json {
    if let Json::Object(fields) = &mut event {
        fields.insert("type".to_owned(), Json::Text("session.updated".to_owned()));
    }
    event
}
