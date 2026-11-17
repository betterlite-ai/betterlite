use betterlite_anthropic::stream_state::state::IdSource;
use betterlite_prompts::arguments::json::Object;

use crate::api::messages_stream_split::{AsyncChunkSource, AsyncChunkSplitter};
use crate::api::messages_stream_usage::state::StreamState;
use crate::api::messages_sync_stream::engine::{self, Transition};

use super::error::ChatStreamError;

// The line the async SSE boundary writes before its one error event, with the failure LiteLLM logged there
pub const MID_STREAM_LOG_PREFIX: &str =
    "Anthropic Adapter - mid-stream error, emitting Anthropic error event: ";

// The asynchronous driver: B2434's engine behind an async pull, with every failure handed to the caller
pub struct ChatStreamDriver<S, I> {
    pub state: StreamState<AsyncChunkSplitter<S>>,
    pub ids: I,
}

impl<S, I> ChatStreamDriver<S, I>
where
    S: AsyncChunkSource + Send,
    I: IdSource,
{
    // One call of __anext__: an event, None at the end, or the failure LiteLLM lets its caller see
    pub async fn next_event(&mut self) -> Result<Option<Object>, ChatStreamError> {
        loop {
            if let Transition::Event(event) = engine::transition(&mut self.state) {
                return Ok(Some(event));
            }
            let chunk = match self.state.completion_stream.source.next_chunk().await {
                Ok(Some(chunk)) => chunk,
                Ok(None) => return self.ended(),
                Err(failure) => return Err(ChatStreamError::Stream { failure }),
            };
            if let Some(event) = engine::update(&mut self.state, &chunk, &self.ids)? {
                return Ok(Some(event));
            }
        }
    }

    // The chunk stream ended, so the end-of-stream flush runs before LiteLLM's StopAsyncIteration handler
    fn ended(&mut self) -> Result<Option<Object>, ChatStreamError> {
        if let Some(event) = engine::end_of_stream(&mut self.state)? {
            return Ok(Some(event));
        }
        Ok(engine::stop_iteration(&mut self.state)?)
    }
}
