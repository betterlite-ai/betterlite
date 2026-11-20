use std::error::Error;

use betterlite_models::decisions::Json;

use crate::api::realtime_contracts::config::BaseRealtimeConfig;
use crate::api::realtime_guardrail_policy::TRANSCRIPTION_EVENT_HOOKS;
use crate::api::realtime_guardrail_policy::guardrails_run_for_hooks;
use crate::api::realtime_session_state::TRANSCRIPTION_COMPLETED;
use crate::api::realtime_session_state::store::SessionMessage;
use crate::api::realtime_setup_buffer::SetupBuffer as _;
use crate::api::realtime_setup_buffer::SetupFlush as _;

use super::super::Outbound;
use super::super::RESPONSE_CREATE;
use super::super::RealtimeBackendRelay;
use super::super::error::RealtimeRelayError;
use super::super::event_type;
use super::super::transcript_and_item_id;

// LiteLLM drains the pre-setup buffer until it is empty or a send fails, and always clears the flushing flag afterwards
async fn complete_deferred_setup<Config, CallbackError>(
    relay: &mut RealtimeBackendRelay<'_, Config, CallbackError>,
) -> Result<(), RealtimeRelayError<Config::Error, CallbackError>>
where
    Config: BaseRealtimeConfig,
    CallbackError: Error + Send + 'static,
{
    if !relay.state.uses_deferred_backend_setup() || relay.state.setup.backend_setup_complete.0 {
        return Ok(());
    }
    relay.state.setup.backend_setup_complete.0 = true;
    relay.state.setup.flushing_pending_messages.0 = true;
    let mut drained = Ok(());
    while drained.is_ok() && !relay.state.setup.pending_messages_until_setup.is_empty() {
        let flushed = relay
            .sender()
            .flush_pending_messages()
            .await
            .map_err(RealtimeRelayError::SetupBuffer);
        match flushed {
            Ok(true) => {}
            Ok(false) => break,
            Err(error) => drained = Err(error),
        }
    }
    relay.state.setup.flushing_pending_messages.0 = false;
    drained
}

// The first session.created completes deferred setup; a later one is re-emitted as session.updated because the client already has one
async fn prepare_session_event<Config, CallbackError>(
    relay: &mut RealtimeBackendRelay<'_, Config, CallbackError>,
    mut event: Json,
) -> Result<Json, RealtimeRelayError<Config::Error, CallbackError>>
where
    Config: BaseRealtimeConfig,
    CallbackError: Error + Send + 'static,
{
    complete_deferred_setup(relay).await?;
    if relay.state.setup.session_created_sent_to_client.0 {
        event = retype_as_session_updated(event);
    } else {
        relay.state.setup.session_created_sent_to_client.0 = true;
    }
    Ok(event)
}

// A guardrailed session.created is stored and forwarded before the one-time auto-response disable is attempted
async fn relay_guardrailed_session_created<Config, CallbackError>(
    relay: &mut RealtimeBackendRelay<'_, Config, CallbackError>,
    event: &Json,
    event_str: &str,
) -> Result<(), RealtimeRelayError<Config::Error, CallbackError>>
where
    Config: BaseRealtimeConfig,
    CallbackError: Error + Send + 'static,
{
    relay
        .state
        .store_message(SessionMessage::Text(event_str))
        .map_err(RealtimeRelayError::State)?;
    relay.send(Outbound::Event(event, event_str)).await?;
    relay.send(Outbound::DisableAutoResponse).await
}

// A completed transcript is logged and forwarded before the guardrails see it, and only a clean conversational turn asks for a reply
async fn relay_completed_transcript<Config, CallbackError>(
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
    relay.send(Outbound::Event(event, event_str)).await?;
    let blocked = relay.run_guardrails(&transcript, item_id).await?;
    if !blocked && !relay.state.is_transcription_session.0 {
        relay.send(Outbound::Backend(RESPONSE_CREATE)).await?;
    }
    Ok(())
}

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
        event = prepare_session_event(relay, event).await?;
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
        return relay_guardrailed_session_created(relay, &event, &event_str).await;
    }
    if event_type(&event) == Some(TRANSCRIPTION_COMPLETED) {
        return relay_completed_transcript(relay, &event, &event_str).await;
    }
    relay
        .state
        .store_message(SessionMessage::Text(&event_str))
        .map_err(RealtimeRelayError::State)?;
    relay.send(Outbound::Event(&event, &event_str)).await
}

// LiteLLM re-emits the provider's real session.created as session.updated once a synthetic one reached the client
fn retype_as_session_updated(mut event: Json) -> Json {
    if let Json::Object(fields) = &mut event {
        fields.insert("type".to_owned(), Json::Text("session.updated".to_owned()));
    }
    event
}
