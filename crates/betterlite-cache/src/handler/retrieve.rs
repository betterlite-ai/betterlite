use betterlite_context::local_ttl_cache::CacheValue;
use betterlite_logging::verbose_hooks::{ConsoleSink, LoggerSink};

use crate::cache_key::derive::get_cache_key;
use crate::cache_key::value::{KeyObject, KeyValue};
use crate::envelope::facade::ResponseCache;
use crate::envelope::request::EnvelopeRequest;
use crate::types::call_type::CachingSupportedCallType;
use crate::types::embedding::CachedEmbedding;

use super::HandlerError;
use super::dynamic_cache::DualCacheEnvelope;
use super::embedding::{current_format_embedding_entry, embedding_entry_key, input_items};
use super::kwargs::{
    CACHE_KEY_KWARG, convert_args_to_kwargs, drop_logging_obj_from_kwargs, key_request,
    remove_null_metadata, request_cache_key, request_control, request_stream,
};
use super::state::CachingHandler;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AsyncSupport {
    Supported,
    Unsupported,
}

// LiteLLM asks the cache whether it has async reads before choosing the async or the sync lookup
pub struct CacheSource<'cache, L: LoggerSink, C: ConsoleSink> {
    pub cache: &'cache ResponseCache<L, C>,
    pub async_support: AsyncSupport,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Retrieval {
    Miss,
    Hit(CacheValue),
    Embeddings(Vec<Option<CachedEmbedding>>),
}

pub async fn retrieve_from_cache<L, C>(
    handler: &mut CachingHandler,
    source: Option<&CacheSource<'_, L, C>>,
    call_type: CachingSupportedCallType,
    kwargs: &KeyObject,
    args: &[KeyValue],
) -> Result<Retrieval, HandlerError>
where
    L: LoggerSink + Send + Sync,
    C: ConsoleSink + Send + Sync,
{
    let Some(cache_source) = source else {
        return Ok(Retrieval::Miss);
    };
    let mut new_kwargs = kwargs.clone();
    for (name, value) in convert_args_to_kwargs(&handler.original_function.parameters, args) {
        new_kwargs.insert(name, value);
    }
    remove_null_metadata(&mut new_kwargs);
    if request_stream(&new_kwargs)? && !new_kwargs.contains_key(CACHE_KEY_KWARG) {
        let mut request = key_request(&new_kwargs)?;
        let key = get_cache_key(
            &mut request,
            &cache_source.cache.key_settings,
            &cache_source.cache.hooks,
        )?;
        new_kwargs.insert(CACHE_KEY_KWARG.to_owned(), KeyValue::String(key));
    }
    handler.request_kwargs = drop_logging_obj_from_kwargs(new_kwargs);
    match call_type {
        CachingSupportedCallType::Aembedding => retrieve_embeddings(handler, cache_source).await,
        CachingSupportedCallType::Completion
        | CachingSupportedCallType::Acompletion
        | CachingSupportedCallType::Embedding
        | CachingSupportedCallType::Atranscription
        | CachingSupportedCallType::Transcription
        | CachingSupportedCallType::AtextCompletion
        | CachingSupportedCallType::TextCompletion
        | CachingSupportedCallType::Arerank
        | CachingSupportedCallType::Rerank
        | CachingSupportedCallType::Responses
        | CachingSupportedCallType::Aresponses
        | CachingSupportedCallType::AnthropicMessages
        | CachingSupportedCallType::AanthropicMessages => {
            retrieve_single(handler, cache_source).await
        }
    }
}

async fn retrieve_single<L, C>(
    handler: &mut CachingHandler,
    source: &CacheSource<'_, L, C>,
) -> Result<Retrieval, HandlerError>
where
    L: LoggerSink + Send + Sync,
    C: ConsoleSink + Send + Sync,
{
    let request_cache_key = request_cache_key(&handler.request_kwargs)?;
    let mut read_kwargs = handler.request_kwargs.clone();
    read_kwargs.shift_remove(CACHE_KEY_KWARG);
    let mut request = envelope_request(&read_kwargs)?;
    let preset = match request_cache_key {
        Some(key) => key,
        None => get_cache_key(
            &mut request.key,
            &source.cache.key_settings,
            &source.cache.hooks,
        )?,
    };
    handler.preset_cache_key = Some(preset.clone());
    request.cache_key = Some(preset);
    let cached = read_from_cache(handler, source, &mut request).await;
    Ok(cached.map_or(Retrieval::Miss, Retrieval::Hit))
}

async fn retrieve_embeddings<L, C>(
    handler: &CachingHandler,
    source: &CacheSource<'_, L, C>,
) -> Result<Retrieval, HandlerError>
where
    L: LoggerSink + Send + Sync,
    C: ConsoleSink + Send + Sync,
{
    let items = input_items(&handler.request_kwargs)?;
    let mut results = Vec::with_capacity(items.len());
    for item in &items {
        let key = embedding_entry_key(source.cache, &handler.request_kwargs, item)?;
        let mut request = envelope_request(&handler.request_kwargs)?;
        request.cache_key = Some(key);
        let cached = read_from_cache(handler, source, &mut request).await;
        results.push(cached.and_then(|value| current_format_embedding_entry(&value)));
    }
    if results.iter().all(Option::is_none) {
        return Ok(Retrieval::Miss);
    }
    Ok(Retrieval::Embeddings(results))
}

async fn read_from_cache<L, C>(
    handler: &CachingHandler,
    source: &CacheSource<'_, L, C>,
    request: &mut EnvelopeRequest,
) -> Option<CacheValue>
where
    L: LoggerSink + Send + Sync,
    C: ConsoleSink + Send + Sync,
{
    let envelope = handler
        .dual_cache
        .as_ref()
        .map(|cache| DualCacheEnvelope { cache });
    match (envelope.as_ref(), source.async_support) {
        (Some(dynamic), AsyncSupport::Supported) => {
            source.cache.async_get_cache(Some(dynamic), request).await
        }
        (Some(dynamic), AsyncSupport::Unsupported) => {
            source.cache.get_cache(Some(dynamic), request)
        }
        (None, AsyncSupport::Supported) => source.cache.async_get_cache(None, request).await,
        (None, AsyncSupport::Unsupported) => source.cache.get_cache(None, request),
    }
}

fn envelope_request(kwargs: &KeyObject) -> Result<EnvelopeRequest, HandlerError> {
    Ok(EnvelopeRequest {
        cache_key: None,
        key: key_request(kwargs)?,
        control: request_control(kwargs)?,
    })
}
