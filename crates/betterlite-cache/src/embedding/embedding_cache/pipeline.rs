use async_trait::async_trait;
use betterlite_context::local_ttl_cache::CacheValue;
use betterlite_logging::verbose_hooks::{ConsoleSink, LoggerSink};
use betterlite_redact::structured::Payload;

use crate::backend::{CacheBackend, CacheResult, Ttl};
use crate::cache_key::derive::should_use_cache;
use crate::cache_key::value::KeyValue;
use crate::envelope::facade::ResponseCache;
use crate::envelope::request::EnvelopeRequest;

use super::super::response::EmbeddingResponse;
use super::{EmbeddingCacheEntry, EmbeddingCacheError, add_embedding_entry_to_cache, request_ttl};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativeCacheRequest {
    pub cache_key: String,
}

// LiteLLM's ResponseCacheRuntime: it turns the request into its own native form and stores a whole batch in one call.
#[async_trait]
pub trait NativeEmbeddingStore: Send + Sync {
    fn request(&self, cache_key: &str, request: &EnvelopeRequest) -> Option<NativeCacheRequest>;

    async fn async_store_batch(
        &self,
        requests: Vec<NativeCacheRequest>,
        responses: Vec<CacheValue>,
    ) -> CacheResult<()>;
}

pub async fn async_add_cache_pipeline<L, C>(
    cache: &ResponseCache<L, C>,
    dynamic: Option<&dyn CacheBackend>,
    native: Option<&dyn NativeEmbeddingStore>,
    request: &mut EnvelopeRequest,
    result: &EmbeddingResponse,
) where
    L: LoggerSink + Send + Sync,
    C: ConsoleSink + Send + Sync,
{
    if !should_use_cache(&cache.key_settings, request.key.cache_control.as_ref()) {
        return;
    }
    if let Err(error) = write_embedding_pipeline(cache, dynamic, native, request, result).await {
        let message = format!("LiteLLM Cache: exception in add_cache: {error}");
        let _logged = cache.hooks.utils_print_verbose(
            Payload::String(message),
            "ERROR",
            &Payload::Bool(true),
        );
    }
}

async fn write_embedding_pipeline<L, C>(
    cache: &ResponseCache<L, C>,
    dynamic: Option<&dyn CacheBackend>,
    native: Option<&dyn NativeEmbeddingStore>,
    request: &mut EnvelopeRequest,
    result: &EmbeddingResponse,
) -> Result<(), EmbeddingCacheError>
where
    L: LoggerSink + Send + Sync,
    C: ConsoleSink + Send + Sync,
{
    let input = input_param(request)?;
    if skip_write(cache, &input, result) {
        return Ok(());
    }
    let ttl = request_ttl(cache, request);
    let mut entries = Vec::with_capacity(input_count(&input));
    for (index, item) in input_items(&input).into_iter().enumerate() {
        entries.push(add_embedding_entry_to_cache(
            cache, request, result, item, index,
        )?);
    }
    if let Some(store) = native {
        return native_batch(store, entries, request).await;
    }
    store_entries(cache, dynamic, entries, ttl).await
}

fn input_param(request: &EnvelopeRequest) -> Result<KeyValue, EmbeddingCacheError> {
    request
        .key
        .params
        .iter()
        .find(|param| param.name == "input")
        .map(|param| param.value.clone())
        .ok_or(EmbeddingCacheError::MissingInput)
}

const fn input_count(input: &KeyValue) -> usize {
    match input {
        KeyValue::Array(items) => items.len(),
        KeyValue::Null
        | KeyValue::Bool(_)
        | KeyValue::Number(_)
        | KeyValue::String(_)
        | KeyValue::Object(_) => 1,
    }
}

fn input_items(input: &KeyValue) -> Vec<&KeyValue> {
    match input {
        KeyValue::Array(items) => items.iter().collect(),
        KeyValue::Null
        | KeyValue::Bool(_)
        | KeyValue::Number(_)
        | KeyValue::String(_)
        | KeyValue::Object(_) => vec![input],
    }
}

fn skip_write<L, C>(
    cache: &ResponseCache<L, C>,
    input: &KeyValue,
    result: &EmbeddingResponse,
) -> bool
where
    L: LoggerSink,
    C: ConsoleSink,
{
    let input_count = input_count(input);
    if result.data.len() == input_count {
        return false;
    }
    let message = format!(
        "LiteLLM Cache: skipping embedding cache write, {input_count} inputs but {} embeddings in the response",
        result.data.len()
    );
    let _logged = cache.hooks.logger.debug(&Payload::String(message));
    true
}

async fn store_entries<L, C>(
    cache: &ResponseCache<L, C>,
    dynamic: Option<&dyn CacheBackend>,
    entries: Vec<EmbeddingCacheEntry>,
    ttl: Ttl,
) -> Result<(), EmbeddingCacheError>
where
    L: LoggerSink + Send + Sync,
    C: ConsoleSink + Send + Sync,
{
    let cache_list = entries
        .into_iter()
        .map(|entry| (entry.cache_key, entry.cached_data))
        .collect();
    match dynamic {
        Some(store) => store.async_set_cache_pipeline(cache_list, ttl).await?,
        None => {
            for (key, value) in cache_list {
                cache.backend.async_set_cache(&key, value, ttl).await?;
            }
        }
    }
    Ok(())
}

async fn native_batch(
    store: &dyn NativeEmbeddingStore,
    entries: Vec<EmbeddingCacheEntry>,
    request: &EnvelopeRequest,
) -> Result<(), EmbeddingCacheError> {
    let mut requests = Vec::with_capacity(entries.len());
    let mut responses = Vec::with_capacity(entries.len());
    for entry in entries {
        let Some(created) = store.request(&entry.cache_key, request) else {
            continue;
        };
        requests.push(created);
        responses.push(entry.cached_embedding);
    }
    store
        .async_store_batch(requests, responses)
        .await
        .map_err(EmbeddingCacheError::from)
}
