use betterlite_context::local_ttl_cache::CacheValue;
use betterlite_logging::logging_streams::assembly::StreamAssemblyError;
use betterlite_logging::logging_streams::cache_assembly::{StreamChunks, assemble_complete_stream};
use betterlite_logging::logging_success::types::SuccessResult;
use betterlite_logging::verbose_hooks::{ConsoleSink, LoggerSink};
use betterlite_models::decisions::Json;

use crate::cache_key::value::KeyObject;
use crate::handler::state::CachingHandler;
use crate::handler::value::to_cache_value;
use crate::write_completion::result::CacheableResult;
use crate::write_completion::store::{
    CacheContext, StoreError, StoreRequest, WriteContext, store_response, store_response_sync,
};

// What LiteLLM hands stream_chunk_builder at end of stream: the handler's start time, now, the request kwargs and the chunks.
pub struct StreamAssembly<'a> {
    pub start_time: f64,
    pub end_time: f64,
    pub request_kwargs: &'a KeyObject,
    pub chunks: &'a [CacheValue],
    pub is_async: bool,
}

// B2227's builder, injected: the cache crate buffers chunks and asks the logging side to turn them into one response.
pub trait CacheStreamAssembler: Send + Sync {
    fn assemble(&self, request: &StreamAssembly<'_>) -> Result<Option<Json>, StreamAssemblyError>;
}

struct CacheStreamChunks<'a> {
    chunks: &'a mut Vec<CacheValue>,
    assembler: &'a dyn CacheStreamAssembler,
    start_time: f64,
    end_time: f64,
    request_kwargs: &'a KeyObject,
    is_async: bool,
}

// LiteLLM's _add_streaming_response_to_cache: the buffer grows until the chunk with a finish reason, and only its assembly is stored.
pub fn add_streaming_response_to_cache<L, C>(
    result: &SuccessResult,
    handler: &mut CachingHandler,
    assembler: &dyn CacheStreamAssembler,
    now: f64,
    context: &WriteContext<L, C>,
) -> Result<(), StoreError>
where
    L: LoggerSink + Send + Sync + 'static,
    C: ConsoleSink + Send + Sync + 'static,
{
    let mut chunks = CacheStreamChunks {
        chunks: &mut handler.async_streaming_chunks,
        assembler,
        start_time: handler.start_time,
        end_time: now,
        request_kwargs: &handler.request_kwargs,
        is_async: true,
    };
    let chunk = to_cache_value(result.response()).ok_or(StoreError::Unrepresentable)?;
    let Some(complete) = assemble_complete_stream(result, &chunk, &mut chunks) else {
        return Ok(());
    };
    store_response(
        context,
        handler.dual_cache.as_ref(),
        &StoreRequest {
            result: &CacheableResult::Chat(complete),
            original_function: &handler.original_function,
            kwargs: &handler.request_kwargs,
            args: &[],
        },
    )
}

pub fn add_sync_streaming_to_cache<L, C>(
    result: &SuccessResult,
    handler: &mut CachingHandler,
    assembler: &dyn CacheStreamAssembler,
    now: f64,
    context: &CacheContext<L, C>,
) -> Result<(), StoreError>
where
    L: LoggerSink + Send + Sync,
    C: ConsoleSink + Send + Sync,
{
    let mut chunks = CacheStreamChunks {
        chunks: &mut handler.sync_streaming_chunks,
        assembler,
        start_time: handler.start_time,
        end_time: now,
        request_kwargs: &handler.request_kwargs,
        is_async: false,
    };
    let chunk = to_cache_value(result.response()).ok_or(StoreError::Unrepresentable)?;
    let Some(complete) = assemble_complete_stream(result, &chunk, &mut chunks) else {
        return Ok(());
    };
    store_response_sync(
        context,
        &StoreRequest {
            result: &CacheableResult::Chat(complete),
            original_function: &handler.original_function,
            kwargs: &handler.request_kwargs,
            args: &[],
        },
    )
}

impl StreamChunks<CacheValue> for CacheStreamChunks<'_> {
    fn push(&mut self, chunk: &CacheValue) {
        self.chunks.push(chunk.clone());
    }

    fn build(&self) -> Result<Option<Json>, StreamAssemblyError> {
        self.assembler.assemble(&StreamAssembly {
            start_time: self.start_time,
            end_time: self.end_time,
            request_kwargs: self.request_kwargs,
            chunks: self.chunks.as_slice(),
            is_async: self.is_async,
        })
    }
}
