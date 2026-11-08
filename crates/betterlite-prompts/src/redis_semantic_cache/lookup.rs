use std::ops::Sub as _;

use serde_json::Number;

use super::RedisSemanticCache;
use super::SIMILARITY_FIELD;
use super::SemanticCacheKwargs;
use super::error::RedisSemanticError;
use super::index;
use super::ports::{Hit, SemanticSearchQuery};
use crate::arguments::json::{Json, Object};
use crate::semantic_cache_records::{decode_cached_response, extract_semantic_prompt};

pub(crate) async fn lookup(
    cache: &RedisSemanticCache,
    key: &str,
    kwargs: &SemanticCacheKwargs,
    metadata: &mut Option<&mut Object>,
) -> Result<Option<Json>, RedisSemanticError> {
    let messages = kwargs
        .messages
        .as_deref()
        .filter(|messages| !messages.is_empty());
    let Some(prompt) = extract_semantic_prompt(messages, kwargs.input.as_ref())? else {
        tracing::debug!("No prompt provided for semantic cache lookup");
        set_zero_similarity(metadata);
        return Ok(None);
    };
    let vector = cache
        .embedder
        .embed(&prompt, kwargs.metadata.as_ref())
        .await?;
    let index = cache
        .index
        .get_or_try_init(|| index::build_index(cache))
        .await?;
    if vector.len() != index.dimension {
        return Err(RedisSemanticError::DimensionMismatch {
            actual: vector.len(),
            expected: index.dimension,
        });
    }
    let query = SemanticSearchQuery {
        vector,
        distance_threshold: cache.distance_threshold,
        cache_key: key.to_owned(),
    };
    let hits = index.backend.ft_search(&index.name, &query).await?;
    read_hit(hits.first(), key, metadata)
}

fn read_hit(
    hit: Option<&Hit>,
    key: &str,
    metadata: &mut Option<&mut Object>,
) -> Result<Option<Json>, RedisSemanticError> {
    let Some(candidate) = hit else {
        set_zero_similarity(metadata);
        return Ok(None);
    };
    if !RedisSemanticCache::cache_hit_matches_key(candidate, key) {
        tracing::debug!("Redis semantic-cache hit did not match cache key scope");
        set_zero_similarity(metadata);
        return Ok(None);
    }
    let Some(distance) = candidate.vector_distance else {
        return Err(RedisSemanticError::MalformedSearchReply);
    };
    let Some(response) = candidate.response.as_deref() else {
        return Err(RedisSemanticError::MalformedSearchReply);
    };
    write_similarity(metadata, 1.0_f64.sub(distance))?;
    match decode_cached_response(Some(response.as_bytes())) {
        Ok(value) => Ok(value),
        Err(error) => {
            tracing::debug!("Error parsing cached response: {error}");
            Ok(None)
        }
    }
}

fn write_similarity(
    metadata: &mut Option<&mut Object>,
    similarity: f64,
) -> Result<(), RedisSemanticError> {
    let Some(target) = metadata.as_deref_mut() else {
        return Ok(());
    };
    let Some(number) = Number::from_f64(similarity) else {
        return Err(RedisSemanticError::NonFiniteSimilarity { similarity });
    };
    target.insert(SIMILARITY_FIELD.to_owned(), Json::Number(number));
    Ok(())
}

pub(crate) fn set_zero_similarity(metadata: &mut Option<&mut Object>) {
    let Some(target) = metadata.as_deref_mut() else {
        return;
    };
    let Some(number) = Number::from_f64(0.0_f64) else {
        return;
    };
    target.insert(SIMILARITY_FIELD.to_owned(), Json::Number(number));
}
