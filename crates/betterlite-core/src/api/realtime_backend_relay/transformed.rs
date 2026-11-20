pub(super) mod events;
pub(super) mod guardrails;
pub(super) mod send;
pub(super) mod setup;

use std::error::Error;

use betterlite_models::decisions::Json;

use crate::api::realtime_contracts::config::BaseRealtimeConfig;
use crate::api::realtime_contracts::events::RealtimeResponseTransformInput;
use crate::api::realtime_contracts::events::RealtimeResponseTypedDict;
use crate::api::realtime_contracts::transport::RealtimeMessage;
use crate::api::realtime_session_state::state::ResponseAssembly;

use self::events::provider_event;
use super::RealtimeBackendRelay;
use super::error::RealtimeRelayError;

pub(super) async fn handle_provider_config_message<Config, CallbackError>(
    relay: &mut RealtimeBackendRelay<'_, Config, CallbackError>,
    provider: &Config,
    raw_response: &str,
) -> Result<(), RealtimeRelayError<Config::Error, CallbackError>>
where
    Config: BaseRealtimeConfig,
    CallbackError: Error + Send + 'static,
{
    let input = transform_input(&relay.state.response);
    let returned = provider
        .transform_realtime_response(
            &RealtimeMessage::Text(raw_response.to_owned()),
            &relay.state.model,
            &input,
        )
        .map_err(RealtimeRelayError::Transform)?;
    let events = match apply_response(&mut relay.state.response, returned) {
        Json::List(events) => events,
        event
        @ (Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_)) => {
            vec![event]
        }
    };
    for event in events {
        provider_event(relay, event).await?;
    }
    Ok(())
}

// The provider sees every response field current as of the frame it is about to transform
fn transform_input(response: &ResponseAssembly) -> RealtimeResponseTransformInput {
    RealtimeResponseTransformInput {
        session_configuration_request: response.session_configuration_request.clone(),
        current_output_item_id: response.current_output_item_id.clone(),
        current_response_id: response.current_response_id.clone(),
        current_delta_chunks: response.current_delta_chunks.clone(),
        current_item_chunks: response.current_item_chunks.clone(),
        current_conversation_id: response.current_conversation_id.clone(),
        current_delta_type: response.current_delta_type,
    }
}

// LiteLLM replaces all seven fields with the transform's answer, including the None values, and relays its response
fn apply_response(response: &mut ResponseAssembly, returned: RealtimeResponseTypedDict) -> Json {
    response.current_output_item_id = returned.current_output_item_id;
    response.current_response_id = returned.current_response_id;
    response.current_delta_chunks = returned.current_delta_chunks;
    response.current_conversation_id = returned.current_conversation_id;
    response.current_item_chunks = returned.current_item_chunks;
    response.current_delta_type = returned.current_delta_type;
    response.session_configuration_request = returned.session_configuration_request;
    returned.response
}
