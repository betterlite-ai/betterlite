use thiserror::Error;
use uuid::Uuid;

use crate::provider_chunk_decode::chunk::{
    DecodeState, ProviderChunk, ProviderKind, RequestOptions,
};
use crate::provider_chunk_decode::creator::chunk_creator;
use crate::provider_chunk_decode::error::ProviderChunkError;
use crate::stream_chunk_shaping::ChunkShaper;
use crate::stream_chunk_shaping::creation::{StreamInstant, build_chunk};
use crate::stream_chunk_shaping::decision::is_delta_empty;
use crate::stream_chunk_shaping::records::{
    ChunkArguments, JsonObject, JsonValue, StreamChunk, StreamOptions,
};
use crate::stream_chunk_shaping::tokens::{check_send_stream_usage, finish_reason_handler};
use crate::stream_chunk_shaping::usage_totals::calculate_total_usage;

#[derive(Debug, Error)]
pub enum WrapperError {
    #[error(transparent)]
    Chunk(#[from] ProviderChunkError),
    #[error(transparent)]
    Usage(#[from] serde_json::Error),
}

pub struct StreamSetup {
    pub model: String,
    pub stream_options: Option<StreamOptions>,
    pub merge_reasoning_in_choices: bool,
    pub is_function_call: bool,
    pub base_hidden_params: JsonObject,
    pub repeated_chunk_limit: usize,
    pub now: fn() -> StreamInstant,
    pub next_id: fn() -> Uuid,
}

// Everything CustomStreamWrapper keeps between chunks, minus the bytes it reads.
pub struct StreamWrapper {
    pub shaper: ChunkShaper,
    pub decode: DecodeState,
    pub has_stream_options: bool,
    pub sent_stream_usage: bool,
}

impl StreamWrapper {
    #[must_use]
    pub fn new(provider: &str, setup: StreamSetup) -> Self {
        let mut shaper = ChunkShaper::new(
            setup.model.clone(),
            Some(provider.to_owned()),
            setup.merge_reasoning_in_choices,
            setup.base_hidden_params,
        );
        shaper.now = setup.now;
        shaper.next_id = setup.next_id;
        let decode = DecodeState {
            model: setup.model,
            kind: ProviderKind::Builtin,
            received_finish_reason: None,
            intermittent_finish_reason: None,
            sent_first_chunk: false,
            complete_response: String::new(),
            completion_stream: None,
            options: RequestOptions {
                send_stream_usage: check_send_stream_usage(setup.stream_options.as_ref()),
                is_function_call: setup.is_function_call,
            },
            repeated_chunk_limit: setup.repeated_chunk_limit,
            next_id: setup.next_id,
        };
        Self {
            shaper,
            decode,
            has_stream_options: setup.stream_options.is_some(),
            sent_stream_usage: false,
        }
    }

    pub fn push(&mut self, chunk: &ProviderChunk) -> Result<Option<StreamChunk>, WrapperError> {
        let Some(transformed) = chunk_creator(&mut self.shaper, &mut self.decode, chunk)
            .map_err(WrapperError::Chunk)?
        else {
            return Ok(None);
        };
        self.record(transformed)
    }

    // The closing chunks: the finish reason when none went out yet, then the usage chunk when asked for.
    pub fn finish(&mut self) -> Option<Result<StreamChunk, WrapperError>> {
        if self.shaper.progress.sent_last_chunk {
            if self.sent_stream_usage || !self.decode.options.send_stream_usage {
                return None;
            }
            self.sent_stream_usage = true;
            let mut response = build_chunk(&mut self.shaper, ChunkArguments::default(), None);
            response.usage = Some(calculate_total_usage(&self.shaper.chunks));
            return Some(Ok(response));
        }
        self.shaper.progress.sent_last_chunk = true;
        let mut chunk = finish_reason_handler(&mut self.shaper);
        if !self.has_stream_options
            && let Err(error) = self.attach_usage(&mut chunk)
        {
            return Some(Err(error));
        }
        Some(Ok(chunk))
    }

    // The wrapper keeps every chunk for the totals and sends provider usage only in its closing chunk.
    fn record(&mut self, mut chunk: StreamChunk) -> Result<Option<StreamChunk>, WrapperError> {
        self.shaper.chunks.push(chunk.clone());
        let had_usage = chunk.usage.take().is_some();
        let silent = chunk.provider_specific_fields.is_none()
            && chunk
                .choices
                .iter()
                .all(|choice| is_delta_empty(&choice.delta) && choice.finish_reason.is_none());
        if had_usage && silent {
            return Ok(None);
        }
        if self.shaper.progress.sent_last_chunk && !self.has_stream_options {
            self.attach_usage(&mut chunk)?;
        }
        Ok(Some(chunk))
    }

    // The running total goes on the chunk's hidden params, where litellm's wrapper puts it.
    fn attach_usage(&self, chunk: &mut StreamChunk) -> Result<(), WrapperError> {
        let usage: JsonValue = serde_json::to_value(calculate_total_usage(&self.shaper.chunks))
            .and_then(serde_json::from_value)
            .map_err(WrapperError::Usage)?;
        chunk.hidden_params.insert("usage".to_owned(), usage);
        Ok(())
    }
}

mod tests;
