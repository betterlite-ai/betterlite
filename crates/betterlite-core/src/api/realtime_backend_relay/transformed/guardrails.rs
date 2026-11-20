use std::error::Error;

use crate::api::realtime_contracts::config::BaseRealtimeConfig;
use crate::api::realtime_guardrails::runner::RealtimeGuardrails;
use crate::api::realtime_guardrails::runner::RunRealtimeGuardrails as _;

use super::super::RealtimeBackendRelay;
use super::super::error::RealtimeRelayError;

// B1007's runner on the provider path: the transcription hook set, no pre-block frame and no explicit hooks
pub(crate) async fn run_guardrails<Config, CallbackError>(
    relay: &mut RealtimeBackendRelay<'_, Config, CallbackError>,
    transcript: &str,
    item_id: Option<&str>,
) -> Result<bool, RealtimeRelayError<Config::Error, CallbackError>>
where
    Config: BaseRealtimeConfig,
    CallbackError: Error + Send + 'static,
{
    let mut runner = RealtimeGuardrails {
        state: &mut *relay.state,
        provider: relay.provider,
        guardrails: relay.guardrails,
    };
    let blocked = runner
        .run_realtime_guardrails(transcript, item_id, None, None)
        .await
        .map_err(RealtimeRelayError::Guardrails)?;
    Ok(blocked)
}
