use std::error::Error;

use crate::api::realtime_contracts::config::BaseRealtimeConfig;
use crate::api::realtime_setup_buffer::SetupBuffer as _;
use crate::api::realtime_setup_buffer::SetupFlush as _;

use super::super::RealtimeBackendRelay;
use super::super::error::RealtimeRelayError;

// LiteLLM drains the pre-setup buffer until it is empty or a send fails, and always clears the flushing flag afterwards
pub(super) async fn complete_deferred_setup<Config, CallbackError>(
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
