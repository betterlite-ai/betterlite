use betterlite_anthropic::stream_state::state::IdSource;
use betterlite_prompts::arguments::json::Object;

use crate::api::messages_stream_split::ChunkSource;
use crate::api::messages_stream_split::errors::error_status_and_message;
use crate::api::messages_stream_usage::error::StreamStateError;
use crate::api::messages_stream_usage::state::StreamState;

use super::engine::{self, Transition};

// The line both LiteLLM iterators log a swallowed failure under, before the async one writes its SSE frame.
pub const ADAPTER_LOG_PREFIX: &str = "Anthropic Adapter - ";

// The synchronous driver: the pull loop lives here, and every state change is engine's pure transition.
pub struct SyncDriver<S, I> {
    pub state: StreamState<S>,
    pub ids: I,
}

// A failed pull logs one error line and ends the iteration instead of raising, as LiteLLM's does.
fn swallowed<T>(error: &str) -> Option<T> {
    tracing::error!(kind = "stream_failure", "{}{}", ADAPTER_LOG_PREFIX, error);
    None
}

impl<S, I> SyncDriver<S, I>
where
    S: ChunkSource,
    I: IdSource,
{
    // One call of __next__: an event, or None once the iteration has ended.
    pub fn next_event(&mut self) -> Option<Object> {
        loop {
            match engine::transition(&mut self.state) {
                Transition::Event(event) => return Some(event),
                Transition::Pull => {}
            }
            match ChunkSource::next_chunk(&mut self.state.completion_stream) {
                Ok(Some(chunk)) => match engine::update(&mut self.state, &chunk, &self.ids) {
                    Ok(Some(event)) => return Some(event),
                    Ok(None) => {}
                    Err(error) => return swallowed(&error.to_string()),
                },
                Ok(None) => match self.ended() {
                    Ok(event) => return event,
                    Err(error) => return swallowed(&error.to_string()),
                },
                Err(failure) => return swallowed(&error_status_and_message(&failure).1),
            }
        }
    }

    // The chunk stream ended, so the end-of-stream flush runs before LiteLLM's StopIteration handler.
    fn ended(&mut self) -> Result<Option<Object>, StreamStateError> {
        if let Some(event) = engine::end_of_stream(&mut self.state)? {
            return Ok(Some(event));
        }
        engine::stop_iteration(&mut self.state)
    }
}
