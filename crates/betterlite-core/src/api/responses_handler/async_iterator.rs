use std::error::Error as StdError;

use betterlite_interactions::types::response::InteractionsStreamChunk;

use super::error::InteractionsStreamError;
use super::state::{InteractionsStreamState, IteratorSetup};
use super::streams::AsyncResponsesEventStream;
use crate::api::messages_response_events::StreamFlag;

// LiteLLMResponsesInteractionsStreamingIterator.__anext__: the sync drain order, with every upstream pull awaited.
pub struct InteractionsAsyncIterator<Error>
where
    Error: StdError + Send + Sync + 'static,
{
    pub state: Box<InteractionsStreamState>,
    pub upstream: Box<dyn AsyncResponsesEventStream<Error = Error>>,
}

impl<Error> InteractionsAsyncIterator<Error>
where
    Error: StdError + Send + Sync + 'static,
{
    #[must_use]
    pub fn new(
        upstream: Box<dyn AsyncResponsesEventStream<Error = Error>>,
        setup: &IteratorSetup<'_>,
    ) -> Self {
        Self {
            state: Box::new(InteractionsStreamState::new(setup)),
            upstream,
        }
    }

    pub async fn next_event(
        &mut self,
    ) -> Option<Result<InteractionsStreamChunk, InteractionsStreamError<Error>>> {
        if let Some(event) = self.state.pending_events.pop_front() {
            return Some(Ok(event));
        }
        if self.state.finished.0 {
            return None;
        }
        while let Some(step) = self.upstream.next_event().await {
            let chunk = match step {
                Ok(chunk) => chunk,
                Err(error) => return Some(Err(InteractionsStreamError::Upstream(error))),
            };
            let events = match self.state.events_for_chunk(&chunk) {
                Ok(events) => events,
                Err(error) => return Some(Err(InteractionsStreamError::State(error))),
            };
            if events.is_empty() {
                continue;
            }
            self.state.pending_events.extend(events);
            return self.state.pending_events.pop_front().map(Ok);
        }
        self.state.finished = StreamFlag(true);
        let terminal = self.state.terminal_events_on_eof();
        self.state.pending_events.extend(terminal);
        self.state.pending_events.pop_front().map(Ok)
    }
}
