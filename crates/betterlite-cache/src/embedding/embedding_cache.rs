use std::collections::BTreeMap;

use betterlite_context::local_ttl_cache::CacheValue;
use betterlite_logging::verbose_hooks::{ConsoleSink, LoggerSink};
use betterlite_models::decisions::Extra;
use thiserror::Error;

use crate::backend::{BackendError, Ttl};
use crate::cache_key::derive::get_cache_key;
use crate::cache_key::error::CacheKeyError;
use crate::cache_key::request::{CacheKeyRequest, KeyParam};
use crate::cache_key::value::KeyValue;
use crate::codec::{CacheCodec, CodecError};
use crate::envelope::facade::ResponseCache;
use crate::envelope::request::EnvelopeRequest;
use crate::envelope::{RESPONSE_FIELD, TIMESTAMP_FIELD};
use crate::types::embedding::{CachedEmbedding, EMBEDDING_CACHE_FORMAT_VERSION};

use super::response::{EmbeddingRecord, EmbeddingResponse};

pub mod pipeline;
pub mod tokens;

#[path = "embedding_cache/tests.rs"]
mod tests;

#[derive(Debug, Error)]
pub enum EmbeddingCacheError {
    #[error("the embedding response has no item at index {index}")]
    MissingResponseItem { index: usize },
    #[error("the embedding cache request has no input")]
    MissingInput,
    #[error("the embedding cache key could not be derived")]
    Key {
        #[from]
        source: CacheKeyError,
    },
    #[error("the cached embedding could not be encoded")]
    Encode {
        #[from]
        source: CodecError,
    },
    #[error("the embedding cache write failed")]
    Store {
        #[from]
        source: BackendError,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct EmbeddingCacheEntry {
    pub cache_key: String,
    pub cached_data: CacheValue,
    pub cached_embedding: CacheValue,
    pub ttl: Ttl,
}

// LiteLLM's _convert_to_cached_embedding: a response item, whatever shape it arrived in, becomes one format-version-2 record.
#[must_use]
pub fn convert_to_cached_embedding(
    response: &EmbeddingRecord,
    model: Option<&str>,
    prompt_tokens: Option<i64>,
    prompt_tokens_details: Option<Extra>,
) -> CachedEmbedding {
    CachedEmbedding {
        embedding: response.embedding.clone(),
        index: response.index,
        object: response.object.clone(),
        model: model.map(str::to_owned),
        prompt_tokens,
        prompt_tokens_details,
        format_version: EMBEDDING_CACHE_FORMAT_VERSION,
    }
}

pub fn add_embedding_entry_to_cache<L, C>(
    cache: &ResponseCache<L, C>,
    request: &mut EnvelopeRequest,
    result: &EmbeddingResponse,
    input: &KeyValue,
    idx: usize,
) -> Result<EmbeddingCacheEntry, EmbeddingCacheError>
where
    L: LoggerSink,
    C: ConsoleSink,
{
    let cache_key = entry_key(cache, request, input)?;
    request.cache_key = Some(cache_key.clone());
    let record = result
        .data
        .get(idx)
        .ok_or(EmbeddingCacheError::MissingResponseItem { index: idx })?;
    let cached = convert_to_cached_embedding(
        record,
        result.model.as_deref(),
        tokens::per_item_prompt_tokens(result, idx),
        tokens::per_item_prompt_tokens_details(result, idx),
    );
    let cached_embedding = CacheCodec::encode(&cached)?;
    let mut fields = BTreeMap::new();
    fields.insert(
        TIMESTAMP_FIELD.to_owned(),
        CacheValue::Float((cache.clock)()),
    );
    fields.insert(RESPONSE_FIELD.to_owned(), cached_embedding.clone());
    Ok(EmbeddingCacheEntry {
        cache_key,
        cached_data: CacheValue::Object(fields),
        cached_embedding,
        ttl: request_ttl(cache, request),
    })
}

// Each item keys on its own input from a private copy of the key request, so no preset key stands in for it.
fn entry_key<L, C>(
    cache: &ResponseCache<L, C>,
    request: &EnvelopeRequest,
    input: &KeyValue,
) -> Result<String, CacheKeyError>
where
    L: LoggerSink,
    C: ConsoleSink,
{
    let mut key = CacheKeyRequest {
        params: request.key.params.clone(),
        metadata: request.key.metadata.clone(),
        litellm_metadata: request.key.litellm_metadata.clone(),
        litellm_params: request.key.litellm_params.clone(),
        cache_control: request.key.cache_control.clone(),
    };
    match key.params.iter_mut().find(|param| param.name == "input") {
        Some(param) => param.value = input.clone(),
        None => key.params.push(KeyParam {
            name: "input".to_owned(),
            value: input.clone(),
        }),
    }
    if let Some(params) = key.litellm_params.as_mut() {
        params.preset_cache_key = None;
    }
    get_cache_key(&mut key, &cache.key_settings, &cache.hooks)
}

pub(super) fn request_ttl<L, C>(cache: &ResponseCache<L, C>, request: &EnvelopeRequest) -> Ttl
where
    L: LoggerSink,
    C: ConsoleSink,
{
    request
        .control
        .ttl
        .and_then(|seconds| serde_json::Number::from(seconds).as_f64())
        .or(cache.settings.ttl)
}
