use std::error::Error;

use betterlite_models::decisions::Json;

use crate::api::realtime_contracts::config::BaseRealtimeConfig;
use crate::api::realtime_sender::RealtimeSend as _;

use super::super::RealtimeBackendRelay;
use super::super::error::RealtimeRelayError;

// The three frames the relay sends itself, so a call site names the frame instead of rebuilding the sender around it
pub(crate) enum Outbound<'frame> {
    Event(&'frame Json, &'frame str),
    Backend(&'frame str),
    DisableAutoResponse,
}

pub(crate) async fn send<Config, CallbackError>(
    relay: &mut RealtimeBackendRelay<'_, Config, CallbackError>,
    outbound: Outbound<'_>,
) -> Result<(), RealtimeRelayError<Config::Error, CallbackError>>
where
    Config: BaseRealtimeConfig,
    CallbackError: Error + Send + 'static,
{
    let mut sender = relay.sender();
    let result = match outbound {
        Outbound::Event(event, event_str) => sender
            .send_event_to_client(event, event_str)
            .await
            .map(|_sent| ()),
        Outbound::Backend(message) => sender.send_to_backend(message).await.map(|_sent| ()),
        Outbound::DisableAutoResponse => sender.maybe_disable_auto_response().await,
    };
    result.map_err(RealtimeRelayError::Send)
}
