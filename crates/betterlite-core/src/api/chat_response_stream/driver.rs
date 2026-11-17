use std::collections::VecDeque;
use std::mem::replace;

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
        if !progress.pending_response_events.is_empty() {
            return Ok(Some(progress.pending_response_events.remove(0)));
        }
        if progress.pending_tool_events.is_empty() {
            return Ok(None);
        }
        Ok(Some(progress.pending_tool_events.remove(0).event))
    }

    fn begin_chunk(&mut self, chunk: &Json) -> Result<(), ChatResponseStreamError> {
        let chunk_fields = object(chunk).and_then(|fields| fields.get("provider_specific_fields"));
        let delta_fields =
            chunk::delta(chunk).and_then(|fields| fields.get("provider_specific_fields"));
        for provider_fields in [chunk_fields, delta_fields].into_iter().flatten() {
            if let Json::Object(values) = provider_fields {
                merge_provider_specific_fields(&mut self.state, values);
            }
            reserve_web_search_indexes(&mut self.state, provider_fields)?;
        }
        let fields = object(chunk).ok_or(ChatResponseStreamError::InvalidChunk)?;
        translate::ensure_output_item_for_chunk(&mut self.state, &self.sources, chunk)?;
        self.state.progress.chunk_snapshots.push(fields.clone());
        Ok(())
    }

    fn finish_chunk(&mut self, chunk: &Json) -> EventResult {
        let event =
            translate::transform(&mut self.state, &self.sources, chunk, &mut self.annotations)?;
        let progress = &mut self.state.progress;
        progress.pending_response_events.extend(event);
        if progress.pending_response_events.is_empty() {
            return Ok(None);
        }
        Ok(Some(progress.pending_response_events.remove(0)))
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
        let mut next = None;
        while !self.state.progress.finished && next.is_none() {
            next = self
                .step()
                .inspect_err(|_error| self.state.progress.finished = true)?;
        }
        Ok(next)
    }

    fn step(&mut self) -> EventResult {
        self.prime_id()?;
        if let Some(event) = self.ready_event()? {
            return Ok(Some(event));
        }
        let Some(chunk) = self.pull_chunk()? else {
            return self.end_of_stream();
        };
        self.begin_chunk(&chunk)?;
        self.finish_chunk(&chunk)
    }

    // A buffered chunk comes before the source, and an exhausted stream never reads it again
    fn pull_chunk(&mut self) -> Result<Option<Json>, ChatResponseStreamError> {
        if let Some(buffered) = self.state.progress.buffered_chunk.take() {
            return Ok(Some(buffered));
        }
        if self.state.progress.upstream_exhausted {
            return Ok(None);
        }
        let next = self.source.next_chunk()?;
        self.state.progress.upstream_exhausted |= next.is_none();
        Ok(next)
    }

    // Pull the first chunk before response.created so that event carries the chat completion id
    pub(super) fn prime_id(&mut self) -> PrimeResult {
        if replace(&mut self.state.progress.response_id_primed, true) {
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
        if result.is_err() {
            self.state.progress.finished = true;
        }
        result
    }

    pub(super) async fn prime_id_async(&mut self) -> PrimeResult {
        if replace(&mut self.state.progress.response_id_primed, true) {
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
            let buffered = self.state.progress.buffered_chunk.take();
            let pulled = if buffered.is_some() {
                buffered
            } else if self.state.progress.upstream_exhausted {
                None
            } else {
                let next = self.source.next_chunk().await?;
                self.state.progress.upstream_exhausted |= next.is_none();
                next
            };
            let Some(chunk) = pulled else {
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
}

// True once priming has its chunk or has seen the end; a null chunk is not a provider answer and is skipped
fn settle(progress: &mut StreamProgress, chunk: Option<Json>) -> bool {
    let Some(answer) = chunk else {
        progress.upstream_exhausted = true;
        return true;
    };
    if matches!(answer, Json::Null) {
        return false;
    }
    if progress.cached_response_id.is_none()
        && let Some(Json::Text(id)) = object(&answer).and_then(|fields| fields.get("id"))
        && !id.is_empty()
    {
        progress.cached_response_id = Some(id.clone());
    }
    progress.buffered_chunk = Some(answer);
    true
}
