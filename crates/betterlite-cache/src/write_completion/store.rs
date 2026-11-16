use std::sync::Arc;

use betterlite_context::local_ttl_cache::CacheValue;
use betterlite_logging::verbose_hooks::{ConsoleSink, LoggerSink};
use betterlite_models::decisions::Json;
use thiserror::Error;

use crate::backend::CacheBackend;
use crate::backend::memory::MemoryCacheOps;
use crate::cache_key::value::{KeyObject, KeyValue};
use crate::dual_cache::DualCache;
use crate::embedding::embedding_cache::pipeline::async_add_cache_pipeline;
use crate::embedding::response::EmbeddingResponse;
use crate::envelope::backend::EnvelopeBackend;
use crate::envelope::facade::ResponseCache;
use crate::handler::HandlerError;
use crate::handler::call_types::should_store_result_in_cache;
use crate::handler::state::OriginalFunction;
use crate::handler::value::to_cache_value;
use crate::types::cache_type::LiteLlmCacheType;
use crate::types::call_type::CachingSupportedCallType;
use crate::write_completion::dual::SharedDualCache;
use crate::write_completion::request::{
    CacheWriteRequest, add_parent_otel_span, merged_request_kwargs,
};
use crate::write_completion::result::CacheableResult;
use crate::write_completion::tracker::{CacheWriteFuture, CacheWriteTracker};

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("the cache write request is invalid")]
    Request {
        #[from]
        source: HandlerError,
    },
    #[error("the response could not be serialized for the cache")]
    Serialize {
        #[from]
        source: serde_json::Error,
    },
    #[error("the response has no cache representation")]
    Unrepresentable,
}

// What the store reads off the configured cache: the facade itself and the call types it was built to cache.
pub struct CacheContext<L: LoggerSink, C: ConsoleSink> {
    pub cache: Option<Arc<ResponseCache<L, C>>>,
    pub supported_call_types: Option<Vec<CachingSupportedCallType>>,
}

// The async store starts its write on this tracker and returns before the write ends.
pub struct WriteContext<L: LoggerSink, C: ConsoleSink> {
    pub cache: CacheContext<L, C>,
    pub tracker: Arc<CacheWriteTracker>,
}

pub struct StoreRequest<'a> {
    pub result: &'a CacheableResult,
    pub original_function: &'a OriginalFunction,
    pub kwargs: &'a KeyObject,
    pub args: &'a [KeyValue],
}

// LiteLLM's async_set_cache: merge the positional arguments, resolve the parent span, then pick one of the three writes.
pub fn store_response<L, C, Memory>(
    context: &WriteContext<L, C>,
    dual_cache: Option<&DualCache<Memory>>,
    request: &StoreRequest<'_>,
) -> Result<(), StoreError>
where
    L: LoggerSink + Send + Sync + 'static,
    C: ConsoleSink + Send + Sync + 'static,
    Memory: MemoryCacheOps + 'static,
{
    let Some(cache) = context.cache.cache.as_ref() else {
        return Ok(());
    };
    let mut kwargs = merged_request_kwargs(request.kwargs, request.original_function, request.args);
    add_parent_otel_span(&mut kwargs);
    if !should_store_result_in_cache(
        request.original_function.name.as_str(),
        context.cache.supported_call_types.as_deref(),
        &kwargs,
    )? {
        return Ok(());
    }
    let prepared = CacheWriteRequest::prepare(&kwargs)?;
    match request.result {
        CacheableResult::Embedding(response)
            if cache.key_settings.cache_type != LiteLlmCacheType::S3 =>
        {
            start_pipeline_write(cache, &context.tracker, dual_cache, &prepared, response);
        }
        CacheableResult::Chat(_)
        | CacheableResult::Embedding(_)
        | CacheableResult::Transcription(_)
        | CacheableResult::Rerank(_)
        | CacheableResult::Responses(_) => {
            let text = request.result.json_text()?;
            start_write(
                cache,
                &context.tracker,
                dual_cache,
                &prepared,
                CacheValue::Text(text),
            );
        }
        CacheableResult::Other(response) => {
            let value = to_cache_value(response).ok_or(StoreError::Unrepresentable)?;
            start_write(cache, &context.tracker, None, &prepared, value);
        }
    }
    Ok(())
}

// LiteLLM's sync_set_cache: no tracker, no dual cache, no parent span, straight to add_cache.
pub fn store_response_sync<L, C>(
    context: &CacheContext<L, C>,
    request: &StoreRequest<'_>,
) -> Result<(), StoreError>
where
    L: LoggerSink + Send + Sync,
    C: ConsoleSink + Send + Sync,
{
    let Some(cache) = context.cache.as_ref() else {
        return Ok(());
    };
    let kwargs = merged_request_kwargs(request.kwargs, request.original_function, request.args);
    if !should_store_result_in_cache(
        request.original_function.name.as_str(),
        context.supported_call_types.as_deref(),
        &kwargs,
    )? {
        return Ok(());
    }
    let prepared = CacheWriteRequest::prepare(&kwargs)?;
    let value = cache_value(request.result)?;
    let mut envelope = prepared.envelope();
    cache.add_cache(&mut envelope, &value);
    Ok(())
}

// The sync path stores the typed response itself; the async path stores its JSON string instead.
fn cache_value(result: &CacheableResult) -> Result<CacheValue, StoreError> {
    let json: Json = match result {
        CacheableResult::Chat(response)
        | CacheableResult::Transcription(response)
        | CacheableResult::Rerank(response)
        | CacheableResult::Responses(response)
        | CacheableResult::Other(response) => response.clone(),
        CacheableResult::Embedding(response) => {
            let wire = serde_json::to_value(response)?;
            serde_json::from_value(wire)?
        }
    };
    to_cache_value(&json).ok_or(StoreError::Unrepresentable)
}

// The write factory runs at least twice when the event loop shuts down, so its captured request and value stay cloneable.
fn start_write<L, C, Memory>(
    cache: &Arc<ResponseCache<L, C>>,
    tracker: &Arc<CacheWriteTracker>,
    dual_cache: Option<&DualCache<Memory>>,
    request: &CacheWriteRequest,
    value: CacheValue,
) where
    L: LoggerSink + Send + Sync + 'static,
    C: ConsoleSink + Send + Sync + 'static,
    Memory: MemoryCacheOps + 'static,
{
    let cache = Arc::clone(cache);
    let dual = dual_cache.map(SharedDualCache::shared);
    let request = request.clone();
    let factory = move || {
        let cache = Arc::clone(&cache);
        let dual = dual.clone();
        let request = request.clone();
        let value = value.clone();
        let future: CacheWriteFuture = Box::pin(async move {
            let mut envelope = request.envelope();
            let dynamic: Option<&dyn EnvelopeBackend> = match &dual {
                Some(store) => Some(store),
                None => None,
            };
            cache.async_add_cache(dynamic, &mut envelope, &value).await;
            Ok(())
        });
        future
    };
    let _write = tracker.spawn(factory);
}

fn start_pipeline_write<L, C, Memory>(
    cache: &Arc<ResponseCache<L, C>>,
    tracker: &Arc<CacheWriteTracker>,
    dual_cache: Option<&DualCache<Memory>>,
    request: &CacheWriteRequest,
    result: &EmbeddingResponse,
) where
    L: LoggerSink + Send + Sync + 'static,
    C: ConsoleSink + Send + Sync + 'static,
    Memory: MemoryCacheOps + 'static,
{
    let cache = Arc::clone(cache);
    let dual = dual_cache.map(SharedDualCache::shared);
    let request = request.clone();
    let result = result.clone();
    let factory = move || {
        let cache = Arc::clone(&cache);
        let dual = dual.clone();
        let request = request.clone();
        let result = result.clone();
        let future: CacheWriteFuture = Box::pin(async move {
            let mut envelope = request.envelope();
            let dynamic: Option<&dyn CacheBackend> = match &dual {
                Some(store) => Some(store),
                None => None,
            };
            async_add_cache_pipeline(&cache, dynamic, None, &mut envelope, &result).await;
            Ok(())
        });
        future
    };
    let _write = tracker.spawn(factory);
}
