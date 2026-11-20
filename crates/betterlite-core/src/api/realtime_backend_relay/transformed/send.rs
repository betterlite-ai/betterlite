use std::error::Error;

use betterlite_models::decisions::Json;

use crate::api::realtime_contracts::config::BaseRealtimeConfig;
use crate::api::realtime_sender::RealtimeSend as _;
use crate::api::realtime_sender::RealtimeSender;

use super::super::RealtimeBackendRelay;
use super::super::error::RealtimeRelayError;

pub(super) async fn send_event<Config, CallbackError>(
    relay: &mut RealtimeBackendRelay<'_, Config, CallbackError>,
    event: &Json,
    event_str: &str,
) -> Result<(), RealtimeRelayError<Config::Error, CallbackError>>
where
    Config: BaseRealtimeConfig,
    CallbackError: Error + Send + 'static,
{
    let mut sender = RealtimeSender {
        state: &mut *relay.state,
        provider: relay.provider,
        guardrails: &relay.callbacks,
    };
    sender
        .send_event_to_client(event, event_str)
        .await
        .map_err(RealtimeRelayError::Send)?;
    Ok(())
}

pub(crate) async fn send_to_backend<Config, CallbackError>(
    relay: &mut RealtimeBackendRelay<'_, Config, CallbackError>,
    message: &str,
) -> Result<(), RealtimeRelayError<Config::Error, CallbackError>>
where
    Config: BaseRealtimeConfig,
    CallbackError: Error + Send + 'static,
{
    let mut sender = RealtimeSender {
        state: &mut *relay.state,
        provider: relay.provider,
        guardrails: &relay.callbacks,
    };
    sender
        .send_to_backend(message)
        .await
        .map_err(RealtimeRelayError::Send)?;
    Ok(())
}

pub(super) async fn maybe_disable_auto_response<Config, CallbackError>(
    relay: &mut RealtimeBackendRelay<'_, Config, CallbackError>,
) -> Result<(), RealtimeRelayError<Config::Error, CallbackError>>
where
    Config: BaseRealtimeConfig,
    CallbackError: Error + Send + 'static,
{
    let mut sender = RealtimeSender {
        state: &mut *relay.state,
        provider: relay.provider,
        guardrails: &relay.callbacks,
    };
    sender
        .maybe_disable_auto_response()
        .await
        .map_err(RealtimeRelayError::Send)?;
    Ok(())
}
