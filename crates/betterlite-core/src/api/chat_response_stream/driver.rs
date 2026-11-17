use std::collections::VecDeque;

use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_event_records::records::KnownResponseEvent;

use crate::api::chat_response_stream_items::progression::return_default_initial_events;
use crate::api::chat_response_stream_items::snapshot::{
    StreamAssembly, merge_provider_specific_fields,
};
use crate::api::chat_response_stream_items::sources::EventSources;
use crate::api::chat_response_stream_tools::fields::object;
use crate::api::chat_response_stream_tools::state::{ChatStreamState, StreamSetup};
use crate::api::chat_response_stream_tools::web_search::reserve_web_search_indexes;

use super::chunk;
use super::error::ChatResponseStreamError;
use super::finalize;
use super::prime;
use super::source::{AsyncChunkStream, ChunkStream};
use super::translate;

// B923's iterator: the provider chunk stream in, the Responses API events out, one event per pull
pub struct ChatResponseStream<C, Context = LoggingState> {
    pub state: ChatStreamState<Context>,
    pub sources: EventSources,
    pub source: C,
    pub annotations: VecDeque<KnownResponseEvent>,
}

impl<C, Context> ChatResponseStream<C, Context> {
    pub fn new(
        setup: StreamSetup<'_, Context>,
        sources: EventSources,
        source: C,
    ) -> Result<Self, ChatResponseStreamError> {
        Ok(Self {
            state: ChatStreamState::new(setup)?,
            sources,
            source,
            annotations: VecDeque::new(),
        })
    }

    fn ready_event(&mut self) -> Result<Option<KnownResponseEvent>, ChatResponseStreamError> {
        if let Some(event) = return_default_initial_events(&mut self.state, &self.sources)? {
            return Ok(Some(event));
        }
        if let Some(event) = self
            .state
            .progress
            .pending_response_events
            .drain(..1)
            .next()
        {
            return Ok(Some(event));
        }
        Ok(self
            .state
            .progress
            .pending_tool_events
            .drain(..1)
            .next()
            .map(|queued| queued.event))
    }

    fn merge_provider_fields(&mut self, chunk: &Json) -> Result<(), ChatResponseStreamError> {
        let chunk_fields = object(chunk).and_then(|fields| fields.get("provider_specific_fields"));
        let delta_fields =
            chunk::delta(chunk).and_then(|fields| fields.get("provider_specific_fields"));
        for provider_fields in [chunk_fields, delta_fields].into_iter().flatten() {
            if let Json::Object(fields) = provider_fields {
                merge_provider_specific_fields(&mut self.state, fields);
            }
            reserve_web_search_indexes(&mut self.state, provider_fields)?;
        }
        Ok(())
    }

    fn begin_chunk(&mut self, chunk: &Json) -> Result<(), ChatResponseStreamError> {
        let fields = object(chunk).ok_or(ChatResponseStreamError::InvalidChunk)?;
        self.merge_provider_fields(chunk)?;
        translate::ensure_output_item_for_chunk(&mut self.state, &self.sources, chunk)?;
        self.state.progress.chunk_snapshots.push(fields.clone());
        Ok(())
    }

    fn finish_chunk(
        &mut self,
        chunk: &Json,
    ) -> Result<Option<KnownResponseEvent>, ChatResponseStreamError> {
        let event =
            translate::transform(&mut self.state, &self.sources, chunk, &mut self.annotations)?;
        if let Some(event) = event {
            self.state.progress.pending_response_events.push(event);
        }
        Ok(self
            .state
            .progress
            .pending_response_events
            .drain(..1)
            .next())
    }

    fn process(
        &mut self,
        chunk: &Json,
    ) -> Result<Option<KnownResponseEvent>, ChatResponseStreamError> {
        self.begin_chunk(chunk)?;
        self.finish_chunk(chunk)
    }

    fn process_async(
        &mut self,
        chunk: &Json,
    ) -> Result<Option<KnownResponseEvent>, ChatResponseStreamError> {
        self.begin_chunk(chunk)?;
        translate::close_reasoning_segment(&mut self.state, &self.sources, chunk)?;
        self.finish_chunk(chunk)
    }
}

impl<C, Context> ChatResponseStream<C, Context>
where
    Context: StreamAssembly,
{
    // The assembly is the iteration's last step; a null one ends the stream without a completed event
    fn end_of_stream(&mut self) -> Result<Option<KnownResponseEvent>, ChatResponseStreamError> {
        let event = finalize::common_done_event(&mut self.state, &self.sources)?;
        if event.is_none() {
            self.state.progress.finished = true;
        }
        Ok(event)
    }
}

impl<C, Context> ChatResponseStream<C, Context>
where
    C: ChunkStream,
    Context: StreamAssembly,
{
    pub fn next_event(&mut self) -> Result<Option<KnownResponseEvent>, ChatResponseStreamError> {
        self.advance()
            .inspect_err(|_error| self.state.progress.finished = true)
    }

    fn advance(&mut self) -> Result<Option<KnownResponseEvent>, ChatResponseStreamError> {
        loop {
            if self.state.progress.finished {
                return Ok(None);
            }
            prime::prime_response_id(&mut self.state.progress, &mut self.source)?;
            if let Some(event) = self.ready_event()? {
                return Ok(Some(event));
            }
            let Some(chunk) = self.next_chunk()? else {
                return self.end_of_stream();
            };
            if let Some(event) = self.process(&chunk)? {
                return Ok(Some(event));
            }
        }
    }

    fn next_chunk(&mut self) -> Result<Option<Json>, ChatResponseStreamError> {
        if let Some(buffered) = self.state.progress.buffered_chunk.take() {
            return Ok(Some(buffered));
        }
        if self.state.progress.upstream_exhausted {
            return Ok(None);
        }
        self.source
            .next_chunk()
            .map_err(|failure| ChatResponseStreamError::Stream { failure })
    }
}

impl<C, Context> Iterator for ChatResponseStream<C, Context>
where
    C: ChunkStream,
    Context: StreamAssembly,
{
    type Item = Result<KnownResponseEvent, ChatResponseStreamError>;

    fn next(&mut self) -> Option<Self::Item> {
        self.next_event().transpose()
    }
}

impl<C, Context> ChatResponseStream<C, Context>
where
    C: AsyncChunkStream + Send,
    Context: StreamAssembly,
{
    pub async fn next_event_async(
        &mut self,
    ) -> Result<Option<KnownResponseEvent>, ChatResponseStreamError> {
        let result = self.advance_async().await;
        if result.is_err() {
            self.state.progress.finished = true;
        }
        result
    }

    async fn advance_async(
        &mut self,
    ) -> Result<Option<KnownResponseEvent>, ChatResponseStreamError> {
        loop {
            if self.state.progress.finished {
                return Ok(None);
            }
            prime::prime_response_id_async(&mut self.state.progress, &mut self.source).await?;
            if let Some(event) = self.ready_event()? {
                return Ok(Some(event));
            }
            let Some(chunk) = self.next_chunk_async().await? else {
                return self.end_of_stream();
            };
            // The async iterator skips a null chunk; the sync one reads it as an invalid chunk
            if matches!(chunk, Json::Null) {
                continue;
            }
            if let Some(event) = self.process_async(&chunk)? {
                return Ok(Some(event));
            }
        }
    }

    async fn next_chunk_async(&mut self) -> Result<Option<Json>, ChatResponseStreamError> {
        if let Some(buffered) = self.state.progress.buffered_chunk.take() {
            return Ok(Some(buffered));
        }
        if self.state.progress.upstream_exhausted {
            return Ok(None);
        }
        self.source
            .next_chunk()
            .await
            .map_err(|failure| ChatResponseStreamError::Stream { failure })
    }
}
