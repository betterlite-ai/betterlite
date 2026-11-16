use betterlite_core::api::media_relay_logging::deepgram::{
    DeepgramListenOutcome, DeepgramListenRequest, deepgram_listen_outcome,
};
use betterlite_core::api::media_relay_logging::error::MediaRelayLoggingError;
use betterlite_logging::passthrough_stream_logging::object::StreamLoggingObject;
use betterlite_logging::passthrough_stream_logging::types::HandlerOutcome;

use super::write_spend;

// ast-grep-ignore: no-long-names
pub fn deepgram_listen_passthrough_handler(
    logging_obj: &StreamLoggingObject,
    request: &DeepgramListenRequest<'_>,
) -> Result<HandlerOutcome, MediaRelayLoggingError> {
    let DeepgramListenOutcome {
        outcome,
        spend,
        audio_seconds,
        channels,
        billed_seconds,
        unpriced_registry_key,
    } = deepgram_listen_outcome(request)?;
    write_spend(logging_obj, &spend);
    if let Some(key) = &unpriced_registry_key {
        tracing::warn!(registry_key = %key, "deepgram listen passthrough: no registry entry");
    }
    tracing::debug!(
        model = %spend.model,
        audio_seconds = %audio_seconds,
        channels,
        billed_seconds = %billed_seconds,
        response_cost = ?spend.response_cost,
        "deepgram listen passthrough cost tracking",
    );
    Ok(outcome)
}
