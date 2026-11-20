pub mod error;
pub mod source;
mod sse;
mod tests;

use std::collections::VecDeque;
use std::sync::Arc;

use betterlite_anthropic::stream_state::state::IdSource;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use betterlite_redact::secret::Redactor;
use indexmap::IndexMap;

use crate::api::api_boundaries::ApiLimits;
use crate::api::messages_options::INCOMPLETE_STREAM_MESSAGE;
use crate::api::messages_response_events::MessageStreamState;
use crate::api::messages_response_events::error::MessagesResponseEventError;
use crate::api::messages_response_events::failure::fail_with;
use crate::api::messages_response_events::input::{EventAttributes, ParsedEvent};
use crate::api::responses_stream_errors::failure::{FailureCause, FailureKind};

use error::MessagesResponseStreamError;
use source::ResponsesEventSource;

// The wrapper's constructor arguments: the model it stamps on every chunk, the redactor B1119 applies and the wire bounds B1087 resolved.
pub struct MessagesResponseStreamInput {
    pub model: String,
    pub redactor: Option<Arc<Redactor>>,
    pub limits: ApiLimits,
}

// One __anext__ outcome: the next Anthropic chunk, or the end where LiteLLM raises StopAsyncIteration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MessagesResponseStreamResult {
    Chunk(Json),
    End,
}

// AnthropicResponsesStreamWrapper's iteration: B1119's queue and flags, the upstream it pulls, and a cursor past the chunks already emitted.
pub struct MessagesResponseStream<S> {
    state: MessageStreamState,
    source: S,
    model: String,
    pending: VecDeque<Json>,
    emitted: usize,
}

impl<S> MessagesResponseStream<S> {
    // __init__: one msg_ id per stream, recorded on the caller's logging state before any event arrives.
    #[must_use]
    pub fn new(
        source: S,
        ids: &dyn IdSource,
        logging: Option<&mut LoggingState>,
        input: MessagesResponseStreamInput,
    ) -> Self {
        let message_id = format!("msg_{}", ids.next_id());
        if let Some(recorder) = logging {
            recorder.streamed_anthropic_message_id = Some(message_id.clone());
        }
        let model = input.model;
        Self {
            state: MessageStreamState::new(message_id, model.clone(), input.redactor, input.limits),
            source,
            model,
            pending: VecDeque::new(),
            emitted: 0,
        }
    }

    // __anext__: the queued chunk first, then the end of a failed stream, then the one pre-sent message_start.
    pub async fn next_chunk(
        &mut self,
    ) -> Result<MessagesResponseStreamResult, MessagesResponseStreamError>
    where
        S: ResponsesEventSource,
    {
        self.collect();
        if let Some(chunk) = self.pending.pop_front() {
            return Ok(MessagesResponseStreamResult::Chunk(chunk));
        }
        let snapshot = self.state.snapshot();
        if snapshot.stream_failed.0 {
            return Ok(MessagesResponseStreamResult::End);
        }
        if !snapshot.sent_message_start.0 {
            self.start_message()?;
            self.collect();
            return Ok(self.take());
        }
        self.fill().await?;
        self.collect();
        Ok(self.take())
    }

    // async_anthropic_sse_wrapper: one encoded frame per chunk, and None after the last chunk.
    pub async fn next_frame(&mut self) -> Result<Option<Vec<u8>>, MessagesResponseStreamError>
    where
        S: ResponsesEventSource,
    {
        match self.next_chunk().await? {
            MessagesResponseStreamResult::End => Ok(None),
            MessagesResponseStreamResult::Chunk(chunk) => sse::frame(chunk).map(Some),
        }
    }

    // The async for / next() loop: events are processed until one queues a chunk or the upstream raises or ends.
    async fn fill(&mut self) -> Result<(), MessagesResponseStreamError>
    where
        S: ResponsesEventSource,
    {
        loop {
            match self.source.next_event().await {
                Ok(Some(event)) => {
                    self.process(&event)?;
                    self.collect();
                    if !self.pending.is_empty() {
                        return Ok(());
                    }
                }
                Ok(None) => return self.exhausted(),
                Err(failure) => {
                    tracing::error!(
                        "AnthropicResponsesStreamWrapper: upstream Responses stream for {} failed",
                        self.model
                    );
                    fail_with(&mut self.state, &failure)?;
                    self.collect();
                    return Ok(());
                }
            }
        }
    }

    // One pulled event: a mapping is handed to B1119 as Python passes the dict through, any other value as an event without fields.
    fn process(&mut self, event: &Json) -> Result<(), MessagesResponseStreamError> {
        let mapping = match event {
            Json::Object(fields) => Some(fields),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
        };
        if let Err(failure) = self.state.process_event(ParsedEvent {
            attributes: EventAttributes::default(),
            mapping,
        }) {
            tracing::error!(
                "AnthropicResponsesStreamWrapper: upstream Responses stream for {} failed",
                self.model
            );
            fail_with(&mut self.state, &cause_from(failure))?;
            self.collect();
        }
        Ok(())
    }

    // The end-of-stream guard: a stream with no message_stop and no failure gets the 500 incomplete-stream error.
    fn exhausted(&mut self) -> Result<(), MessagesResponseStreamError> {
        let snapshot = self.state.snapshot();
        if !self.pending.is_empty() || snapshot.sent_message_stop.0 || snapshot.stream_failed.0 {
            return Ok(());
        }
        tracing::error!(
            "AnthropicResponsesStreamWrapper: upstream Responses stream for {} ended without a terminal event",
            self.model
        );
        let failure = FailureCause {
            kind: FailureKind::Plain,
            text: INCOMPLETE_STREAM_MESSAGE.to_owned(),
            message: None,
            error_type: None,
            code: None,
            status_code: None,
            body: None,
            original_exception: None,
        };
        fail_with(&mut self.state, &failure)?;
        self.collect();
        Ok(())
    }

    // The pre-sent message_start: B1119 builds the same chunk for response.created, which also marks the flag.
    fn start_message(&mut self) -> Result<(), MessagesResponseStreamError> {
        let mut fields = IndexMap::new();
        fields.insert("type".to_owned(), Json::Text("response.created".to_owned()));
        self.state.process_event(ParsedEvent {
            attributes: EventAttributes::default(),
            mapping: Some(&fields),
        })?;
        Ok(())
    }

    // B1119's queue only grows, so the wrapper keeps a cursor and moves each new chunk into its own queue once.
    fn collect(&mut self) {
        let mut chunks = self.state.snapshot().chunks;
        if chunks.len() <= self.emitted {
            return;
        }
        let unseen = chunks.split_off(self.emitted);
        self.emitted = self.emitted.saturating_add(unseen.len());
        self.pending.extend(unseen);
    }

    // The chunk at the head of the wrapper's queue, or the end when none is queued.
    fn take(&mut self) -> MessagesResponseStreamResult {
        self.pending.pop_front().map_or(
            MessagesResponseStreamResult::End,
            MessagesResponseStreamResult::Chunk,
        )
    }
}

// A rejected event is an exception in __anext__, so its text becomes the status-500 failure message.
fn cause_from(error: MessagesResponseEventError) -> FailureCause {
    FailureCause {
        kind: FailureKind::Plain,
        text: error.to_string(),
        message: None,
        error_type: None,
        code: None,
        status_code: None,
        body: None,
        original_exception: None,
    }
}
