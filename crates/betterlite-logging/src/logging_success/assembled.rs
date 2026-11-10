use betterlite_models::decisions::Json;

use super::types::SuccessResult;
use crate::logging_state::LoggingState;

pub const ASYNC_STREAM_RESPONSE_KEY: &str = "async_complete_streaming_response";
pub const COMPLETE_STREAMING_RESPONSE: &str = "complete_streaming_response";
pub const FINAL_STREAM_DISPATCH_MARKER: &str = "has_dispatched_final_stream_success";

// A chunk or a missing result is a per-chunk success; any other result on a stream is the final export
#[must_use]
pub fn is_assembled_stream_success(state: &LoggingState, result: &SuccessResult) -> bool {
    if state.stream != Some(true) {
        return false;
    }
    match result {
        SuccessResult::Chunk(_) | SuccessResult::Other(Json::Null) => {
            state
                .model_call_details
                .contains_key(ASYNC_STREAM_RESPONSE_KEY)
                || !matches!(
                    state.model_call_details.get(COMPLETE_STREAMING_RESPONSE),
                    None | Some(Json::Null)
                )
        }
        SuccessResult::Chat(_) | SuccessResult::Embeddings(_) | SuccessResult::Other(_) => true,
    }
}

pub fn claim_final_stream_dispatch(state: &mut LoggingState) -> bool {
    if state.model_call_details.get(FINAL_STREAM_DISPATCH_MARKER) == Some(&Json::Bool(true)) {
        return false;
    }
    state
        .model_call_details
        .insert(FINAL_STREAM_DISPATCH_MARKER.to_owned(), Json::Bool(true));
    true
}
