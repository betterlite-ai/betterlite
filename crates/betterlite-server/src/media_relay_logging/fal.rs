use betterlite_core::api::media_relay_logging::error::MediaRelayLoggingError;
use betterlite_core::api::media_relay_logging::fal::{
    FalPassthroughRequest, fal_ai_passthrough_outcome,
};
use betterlite_logging::passthrough_stream_logging::object::StreamLoggingObject;
use betterlite_logging::passthrough_stream_logging::types::HandlerOutcome;

use super::write_spend;

pub fn fal_ai_passthrough_handler(
    logging_obj: &StreamLoggingObject,
    request: &FalPassthroughRequest<'_>,
) -> Result<HandlerOutcome, MediaRelayLoggingError> {
    let outcome = fal_ai_passthrough_outcome(request)?;
    write_spend(logging_obj, &outcome.spend);
    tracing::debug!(
        model = %outcome.spend.model,
        response_cost = ?outcome.spend.response_cost,
        "fal ai passthrough cost tracking",
    );
    Ok(outcome.outcome)
}
