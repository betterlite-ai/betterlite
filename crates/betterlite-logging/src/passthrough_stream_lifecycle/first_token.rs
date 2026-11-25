use crate::logging_state::events::update_completion_start_time;
use crate::logging_success::ports::SuccessClock;
use crate::passthrough_stream_logging::object::StreamLoggingObject;

// LiteLLM's _stamp_first_chunk_if_needed: the first chunk anchors the completion, later chunks never move it
pub fn stamp_first_chunk_if_needed(logging_obj: &StreamLoggingObject, clock: &dyn SuccessClock) {
    let unstamped = logging_obj
        .read(&mut |state| state.completion_start_time.is_none())
        .unwrap_or(false);
    if !unstamped {
        return;
    }
    let now = clock.now();
    logging_obj.edit(&mut |state| update_completion_start_time(state, now));
}

mod tests;
