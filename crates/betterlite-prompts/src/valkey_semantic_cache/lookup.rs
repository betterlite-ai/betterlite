use std::ops::Sub as _;

use serde_json::Number;

use super::ValkeySemanticCache;
use super::error::ValkeySemanticError;
use super::index;
use super::ports::Hit;
use super::schema::{knn_query, pack_vector};
use crate::arguments::json::{Json, Object};
use crate::redis_semantic_cache::{SIMILARITY_FIELD, SemanticCacheKwargs};
use crate::semantic_cache_records::{decode_cached_response, extract_semantic_prompt};

pub(crate) async fn lookup(
    cache: &ValkeySemanticCache,
    key: &str,
    kwargs: &SemanticCacheKwargs,
    metadata: &mut Option<&mut Object>,
) -> Result<Option<Json>, ValkeySemanticError> {
    let messages = kwargs
        .messages
        .as_deref()
        .filter(|messages| !messages.is_empty());
    let Some(prompt) = extract_semantic_prompt(messages, kwargs.input.as_ref())? else {
        set_zero_similarity(metadata);
        return Ok(None);
    };
    let vector = cache
        .embedder
        .embed(&prompt, kwargs.metadata.as_ref())
        .await?;
    let backend = index::backend(cache).await?;
    index::ensure_index(cache, backend.as_ref(), vector.len()).await?;
    let query = knn_query(key);
    let hits = backend
        .ft_search(&cache.index_name, &query, &pack_vector(&vector))
        .await?;
    resolve_hit(cache, hits.first(), metadata)
}

fn resolve_hit(
    cache: &ValkeySemanticCache,
    hit: Option<&Hit>,
    metadata: &mut Option<&mut Object>,
) -> Result<Option<Json>, ValkeySemanticError> {
    let Some(candidate) = hit else {
        set_zero_similarity(metadata);
        return Ok(None);
    };
    let similarity = 1.0_f64.sub(candidate.distance);
    write_similarity(metadata, similarity)?;
    if similarity < cache.similarity_threshold {
        return Ok(None);
    }
    match decode_cached_response(Some(candidate.response.as_bytes())) {
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
) -> Result<(), ValkeySemanticError> {
    let Some(target) = metadata.as_deref_mut() else {
        return Ok(());
    };
    let Some(number) = Number::from_f64(similarity) else {
        return Err(ValkeySemanticError::NonFiniteSimilarity { similarity });
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
