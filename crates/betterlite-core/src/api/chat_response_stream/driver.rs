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
use crate::api::chat_response_stream_tools::state::{ChatStreamState, StreamProgress, StreamSetup};
use crate::api::chat_response_stream_tools::web_search::reserve_web_search_indexes;

use super::chunk;
use super::error::ChatResponseStreamError;
use super::finalize;
use super::source::{AsyncChunkStream, ChunkStream};
use super::translate;

type EventResult = Result<Option<KnownResponseEvent>, ChatResponseStreamError>;
type PrimeResult = Result<(), ChatResponseStreamError>;

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

    fn ready_event(&mut self) -> EventResult {
        if let Some(event) = return_default_initial_events(&mut self.state, &self.sources)? {
            return Ok(Some(event));
        }
        let progress = &mut self.state.progress;
        let pending = progress.pending_response_events.drain(..1).next();
        if pending.is_some() {
            return Ok(pending);
        }
        let tool = progress.pending_tool_events.drain(..1).next();
        Ok(tool.map(|queued| queued.event))
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

    fn finish_chunk(&mut self, chunk: &Json) -> EventResult {
        let event =
            translate::transform(&mut self.state, &self.sources, chunk, &mut self.annotations)?;
        let progress = &mut self.state.progress;
        progress.pending_response_events.extend(event);
        Ok(progress.pending_response_events.drain(..1).next())
    }

    // A buffered chunk comes before the source, and an exhausted stream never reads it again
    fn buffered_chunk(&mut self) -> Option<Result<Option<Json>, ChatResponseStreamError>> {
        if let Some(buffered) = self.state.progress.buffered_chunk.take() {
            return Some(Ok(Some(buffered)));
        }
        self.state.progress.upstream_exhausted.then_some(Ok(None))
    }

    // The assembly is the iteration's last step; a null one ends the stream without a completed event
    fn end_of_stream(&mut self) -> EventResult
    where
        Context: StreamAssembly,
    {
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
    pub fn next_event(&mut self) -> EventResult {
        self.advance()
            .inspect_err(|_error| self.state.progress.finished = true)
    }

    // One turn: a queued event, or the one provider chunk that yields the next event
    fn step(&mut self) -> EventResult {
        self.prime_id()?;
        if let Some(event) = self.ready_event()? {
            return Ok(Some(event));
        }
        let Some(chunk) = self.next_chunk()? else {
            return self.end_of_stream();
        };
        self.begin_chunk(&chunk)?;
        self.finish_chunk(&chunk)
    }

    fn advance(&mut self) -> EventResult {
        let mut next = None;
        while !self.state.progress.finished && next.is_none() {
            next = self.step()?;
        }
        Ok(next)
    }

    fn next_chunk(&mut self) -> Result<Option<Json>, ChatResponseStreamError> {
        if let Some(ready) = self.buffered_chunk() {
            return ready;
        }
        Ok(self.source.next_chunk()?)
    }

    // Pull the first chunk before response.created so that event carries the chat completion id
    pub(super) fn prime_id(&mut self) -> PrimeResult {
        if std::mem::replace(&mut self.state.progress.response_id_primed, true) {
            return Ok(());
        }
        while !settle(&mut self.state.progress, self.source.next_chunk()?) {}
        Ok(())
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
    pub async fn next_event_async(&mut self) -> EventResult {
        let result = self.advance_async().await;
        result.inspect_err(|_error| self.state.progress.finished = true)
    }

    pub(super) async fn prime_id_async(&mut self) -> PrimeResult {
        if std::mem::replace(&mut self.state.progress.response_id_primed, true) {
            return Ok(());
        }
        while !settle(&mut self.state.progress, self.source.next_chunk().await?) {}
        Ok(())
    }

    async fn advance_async(&mut self) -> EventResult {
        while !self.state.progress.finished {
            self.prime_id_async().await?;
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
            self.begin_chunk(&chunk)?;
            translate::close_reasoning_segment(&mut self.state, &self.sources, &chunk)?;
            if let Some(event) = self.finish_chunk(&chunk)? {
                return Ok(Some(event));
            }
        }
        Ok(None)
    }

    async fn next_chunk_async(&mut self) -> Result<Option<Json>, ChatResponseStreamError> {
        if let Some(ready) = self.buffered_chunk() {
            return ready;
        }
        Ok(self.source.next_chunk().await?)
    }
}

// True once priming has its chunk or has seen the end; a null chunk is not a provider answer and is skipped
fn settle(progress: &mut StreamProgress, chunk: Option<Json>) -> bool {
    let Some(chunk) = chunk else {
        progress.upstream_exhausted = true;
        return true;
    };
    if matches!(chunk, Json::Null) {
        return false;
    }
    if progress.cached_response_id.is_none()
        && let Some(Json::Text(id)) = object(&chunk).and_then(|fields| fields.get("id"))
        && !id.is_empty()
    {
        progress.cached_response_id = Some(id.clone());
    }
    progress.buffered_chunk = Some(chunk);
    true
}
