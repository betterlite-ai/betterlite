use betterlite_core::api::media_relay_logging::spend::MediaSpend;
use betterlite_logging::passthrough_logging_state::dispatch::PassThroughLoggingKwargs;
use betterlite_logging::passthrough_logging_state::response::PassThroughResponse;
use betterlite_logging::passthrough_stream_logging::object::StreamLoggingObject;
use betterlite_logging::passthrough_stream_logging::types::HandlerOutcome;

use crate::media_relay_logging::write_spend;

// LiteLLM also writes the provider onto the logging object, which the shared spend logger leaves to the relay route.
pub(super) fn outcome(
    logging_obj: &StreamLoggingObject,
    spend: &MediaSpend,
    result: PassThroughResponse,
    kwargs: &PassThroughLoggingKwargs,
) -> HandlerOutcome {
    write_spend(logging_obj, spend);
    let provider = spend.custom_llm_provider.to_owned();
    logging_obj.edit(&mut |state| state.custom_llm_provider = Some(provider.clone()));
    HandlerOutcome {
        result: Some(result),
        kwargs: spend.kwargs(kwargs),
    }
}
