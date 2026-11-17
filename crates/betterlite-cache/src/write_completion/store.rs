use std::sync::Arc;

use betterlite_context::call_context::{CallContext, CallState};
use betterlite_context::local_ttl_cache::CacheValue;
use betterlite_logging::verbose_hooks::{ConsoleSink, LoggerSink};
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

// The async store starts its write on this tracker and returns before the write ends; the write keeps the call's post-response phase.
pub struct WriteContext<'call, L: LoggerSink, C: ConsoleSink> {
    pub cache: CacheContext<L, C>,
    pub tracker: Arc<CacheWriteTracker>,
    pub call: &'call CallContext<'call>,
}

pub struct StoreRequest<'request> {
    pub result: &'request CacheableResult,
    pub original_function: &'request OriginalFunction,
    pub kwargs: &'request KeyObject,
    pub args: &'request [KeyValue],
}

// LiteLLM's async_set_cache: merge the positional arguments, resolve the parent span, then pick one of the three writes.
pub fn store_response<L, C, Memory>(
    context: &WriteContext<'_, L, C>,
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
    let target = write_target(
        cache,
        dual_cache.map(SharedDualCache::shared),
        request.result,
    )?;
    start_write(cache, context, &prepared, target);
    Ok(())
}

// LiteLLM's async_set_cache routing: an embedding takes the pipeline unless the cache is S3, and anything outside the five response models skips the dual cache.
fn write_target<L, C, Memory>(
    cache: &ResponseCache<L, C>,
    dual: Option<SharedDualCache<Memory>>,
    result: &CacheableResult,
) -> Result<WriteTarget<Memory>, StoreError>
where
    L: LoggerSink,
    C: ConsoleSink,
    Memory: MemoryCacheOps,
{
    if let CacheableResult::Embedding(response) = result
        && cache.key_settings.cache_type != LiteLlmCacheType::S3
    {
        return Ok(WriteTarget::Embedding {
            result: response.clone(),
            dual,
        });
    }
    let skips_dual = matches!(
        result,
        CacheableResult::TextCompletion(_) | CacheableResult::Other(_)
    );
    Ok(WriteTarget::Response {
        value: stored_value(result)?,
        dual: if skips_dual { None } else { dual },
    })
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
    let value = stored_value(request.result)?;
    let mut envelope = prepared.envelope();
    cache.add_cache(&mut envelope, &value);
    Ok(())
}

// LiteLLM's _add_cache_logic serializes every typed response with model_dump_json; anything else is stored as it is.
fn stored_value(result: &CacheableResult) -> Result<CacheValue, StoreError> {
    let CacheableResult::Other(response) = result else {
        return Ok(CacheValue::Text(result.json_text()?));
    };
    to_cache_value(response).ok_or(StoreError::Unrepresentable)
}

enum WriteTarget<Memory: MemoryCacheOps> {
    Response {
        value: CacheValue,
        dual: Option<SharedDualCache<Memory>>,
    },
    Embedding {
        result: EmbeddingResponse,
        dual: Option<SharedDualCache<Memory>>,
    },
}

// The write factory can run a second time when the event loop shuts down, so it captures owned copies of everything.
fn start_write<L, C, Memory>(
    response_cache: &Arc<ResponseCache<L, C>>,
    context: &WriteContext<'_, L, C>,
    write_request: &CacheWriteRequest,
    write_target: WriteTarget<Memory>,
) where
    L: LoggerSink + Send + Sync + 'static,
    C: ConsoleSink + Send + Sync + 'static,
    Memory: MemoryCacheOps + 'static,
{
    let cache = Arc::clone(response_cache);
    let call = context.call;
    let request = write_request.clone();
    let target = Arc::new(write_target);
    let factory = move |_call: CallState| -> CacheWriteFuture {
        let task_cache = Arc::clone(&cache);
        let task_request = request.clone();
        let task_target = Arc::clone(&target);
        Box::pin(async move {
            let mut envelope = task_request.envelope();
            match task_target.as_ref() {
                WriteTarget::Response { value, dual } => {
                    let dynamic: Option<&dyn EnvelopeBackend> =
                        dual.as_ref().map(|store| -> &dyn EnvelopeBackend { store });
                    task_cache
                        .async_add_cache(dynamic, &mut envelope, value)
                        .await;
                }
                WriteTarget::Embedding { result, dual } => {
                    let dynamic: Option<&dyn CacheBackend> =
                        dual.as_ref().map(|store| -> &dyn CacheBackend { store });
                    async_add_cache_pipeline(&task_cache, dynamic, None, &mut envelope, result)
                        .await;
                }
            }
            Ok(())
        })
    };
    let _write = context.tracker.spawn(call, factory);
}
