use std::error::Error;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::api::realtime_contracts::config::BaseRealtimeConfig;
use crate::api::realtime_session_state::store::SessionMessage;

use super::super::RealtimeBackendRelay;
use super::super::error::RealtimeRelayError;

// The event is logged and reaches the client as the normalizer writes it, staying in the frame's own field order
pub(super) async fn store_and_forward_raw<Config, CallbackError>(
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
    let encoded = relay
        .sender()
        .event_to_client_json(Json::Object(fields.clone()))
        .map_err(RealtimeRelayError::Send)?;
    relay.send_client_text(&encoded).await
}
