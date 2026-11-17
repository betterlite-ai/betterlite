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
pub struct StreamAssembly<'stream> {
    pub start_time: f64,
    pub end_time: f64,
    pub request_kwargs: &'stream KeyObject,
    pub chunks: &'stream [CacheValue],
    pub is_async: bool,
}

// B2227's builder, injected: the cache crate buffers chunks and asks the logging side to turn them into one response.
pub trait CacheStreamAssembler: Send + Sync {
    fn assemble(&self, request: &StreamAssembly<'_>) -> Result<Option<Json>, StreamAssemblyError>;
}

struct CacheStreamChunks<'buffer> {
    chunks: &'buffer mut Vec<CacheValue>,
    assembler: &'buffer dyn CacheStreamAssembler,
    start_time: f64,
    end_time: f64,
    request_kwargs: &'buffer KeyObject,
    is_async: bool,
}

// One finished chunk: buffered while it has no finish reason, and assembled into the response to store once it has one.
fn finish_stream(
    result: &SuccessResult,
    handler: &mut CachingHandler,
    assembler: &dyn CacheStreamAssembler,
    now: f64,
    is_async: bool,
) -> Result<Option<Json>, StoreError> {
    let chunks = if is_async {
        &mut handler.async_streaming_chunks
    } else {
        &mut handler.sync_streaming_chunks
    };
    let mut buffer = CacheStreamChunks {
        chunks,
        assembler,
        start_time: handler.start_time,
        end_time: now,
        request_kwargs: &handler.request_kwargs,
        is_async,
    };
    let Some(chunk) = to_cache_value(result.response()) else {
        return Err(StoreError::Unrepresentable);
    };
    Ok(assemble_complete_stream(result, &chunk, &mut buffer))
}

// LiteLLM's stream builder returns a TextCompletionResponse for a text completion, which the store writes as a bare JSON string.
fn assembled_result(body: Json) -> CacheableResult {
    let text_completion = matches!(&body, Json::Object(fields)
        if matches!(fields.get("object"), Some(Json::Text(object)) if object == "text_completion"));
    if text_completion {
        return CacheableResult::TextCompletion(body);
    }
    CacheableResult::Chat(body)
}

// ast-grep-ignore: no-long-names - LiteLLM's _add_streaming_response_to_cache: only the assembled final chunk is stored.
pub fn add_streaming_response_to_cache<L, C>(
    result: &SuccessResult,
    handler: &mut CachingHandler,
    assembler: &dyn CacheStreamAssembler,
    now: f64,
    context: &WriteContext<'_, L, C>,
) -> Result<(), StoreError>
where
    L: LoggerSink + Send + Sync + 'static,
    C: ConsoleSink + Send + Sync + 'static,
{
    let Some(complete) = finish_stream(result, handler, assembler, now, true)? else {
        return Ok(());
    };
    let stored = assembled_result(complete);
    let request = StoreRequest {
        result: &stored,
        original_function: &handler.original_function,
        kwargs: &handler.request_kwargs,
        args: &[],
    };
    store_response(context, handler.dual_cache.as_ref(), &request)
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
    match finish_stream(result, handler, assembler, now, false)? {
        Some(complete) => {
            let stored = assembled_result(complete);
            store_response_sync(
                context,
                &StoreRequest {
                    result: &stored,
                    original_function: &handler.original_function,
                    kwargs: &handler.request_kwargs,
                    args: &[],
                },
            )
        }
        None => Ok(()),
    }
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
