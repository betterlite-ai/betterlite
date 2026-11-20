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
use crate::api::messages_options::refusal::OutputItem;
use crate::api::messages_response_events::MessageStreamState;
use crate::api::messages_response_events::error::MessagesResponseEventError;
use crate::api::messages_response_events::failure::fail_with;
use crate::api::messages_response_events::input::{
    ErrorAttribute, EventAttributes, ParsedEvent, ResponseAttribute, ResponseAttributes,
};
use crate::api::responses_stream_errors::failure::{FailureCause, FailureKind};
use crate::api::responses_stream_primitives::error_fields::StreamErrorAttributes;

use error::MessagesResponseStreamError;
use source::{ProviderEvent, ResponsesEvent, ResponsesEventSource};

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
        let result = self.next_chunk().await?;
        match result {
            MessagesResponseStreamResult::Chunk(chunk) => sse::frame(chunk).map(Some),
            MessagesResponseStreamResult::End => Ok(None),
        }
    }

    // The async for / next() loop: events are processed until one queues a chunk or the upstream raises or ends.
    async fn fill(&mut self) -> Result<(), MessagesResponseStreamError>
    where
        S: ResponsesEventSource,
    {
        while self.pending.is_empty() {
            let event = match self.source.next_event().await {
                Ok(Some(event)) => event,
                Ok(None) => return self.exhausted(),
                Err(failure) => return self.failed_pull(&failure),
            };
            self.process(&event)?;
            self.collect();
        }
        Ok(())
    }

    // One pulled event: the attributes getattr reads decide a provider event, and a dict-shaped event also passes its mapping through.
    fn process(&mut self, event: &ResponsesEvent) -> Result<(), MessagesResponseStreamError> {
        let output = event
            .attributes
            .response
            .as_ref()
            .map_or_else(Vec::new, |response| {
                response.output.iter().map(OutputItem::Raw).collect()
            });
        let attributes = provider_attributes(&event.attributes, &output);
        let failure = self
            .state
            .process_event(ParsedEvent {
                attributes,
                mapping: event.mapping.as_ref(),
            })
            .err();
        if let Some(error) = failure {
            tracing::error!(
                "AnthropicResponsesStreamWrapper: upstream Responses stream for {} failed",
                self.model
            );
            fail_with(&mut self.state, &cause_from(&error))?;
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

    // An upstream exception is logged and becomes the terminal error chunk with its status and message.
    fn failed_pull(&mut self, failure: &FailureCause) -> Result<(), MessagesResponseStreamError> {
        tracing::error!(
            "AnthropicResponsesStreamWrapper: upstream Responses stream for {} failed",
            self.model
        );
        fail_with(&mut self.state, failure)?;
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

// The attributes of a provider event, whose response borrows the output items handed to B1119 alongside it.
fn provider_attributes<'event>(
    prepared: &'event ProviderEvent,
    output: &'event [OutputItem<'event>],
) -> EventAttributes<'event> {
    let response = prepared.response.as_ref().map(|response| {
        let error = response.error.as_ref().map(|failure| {
            ErrorAttribute::Attributes(StreamErrorAttributes {
                message: failure.message.as_ref(),
                error_type: failure.error_type.as_ref(),
                code: failure.code.as_ref(),
            })
        });
        ResponseAttribute::Object(ResponseAttributes {
            output,
            status: response.status.as_deref(),
            usage: response.usage.as_ref(),
            error,
        })
    });
    EventAttributes {
        event_type: prepared.event_type.as_ref(),
        item: prepared.item.as_ref(),
        item_id: prepared.item_id.as_ref(),
        delta: prepared.delta.as_ref(),
        summary_index: prepared.summary_index.as_ref(),
        response,
    }
}

// A rejected event is an exception in __anext__, so its text becomes the status-500 failure message.
fn cause_from(error: &MessagesResponseEventError) -> FailureCause {
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
