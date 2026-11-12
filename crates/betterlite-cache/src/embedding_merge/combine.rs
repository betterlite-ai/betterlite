use std::slice::Iter;

use betterlite_models::decisions::Json;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;

use super::error::MergeError;
use super::response::{
    ApiEmbeddingResponse, CachedEmbeddingResponse, Embedding, MergedEmbeddingResponse,
};
use super::usage::{Usage, combine_usage};

// LiteLLM's _combine_cached_embedding_response_with_api_result; None means the API response stands as it is
pub fn merge_with_api_result(
    cached_response: Option<CachedEmbeddingResponse>,
    fresh: &ApiEmbeddingResponse,
    start_time: DateTime<Utc>,
    end_time: DateTime<Utc>,
) -> Result<Option<MergedEmbeddingResponse>, MergeError> {
    let Some(cached) = cached_response else {
        return Ok(None);
    };
    let usage = match (&cached.usage, &fresh.usage) {
        (Some(cached_usage), Some(fresh_usage)) => combine_usage(cached_usage, fresh_usage)?,
        (Some(only), None) => only.clone(),
        (None, Some(_) | None) => Usage::default(),
    };
    let mut fresh_items = fresh.data.iter();
    let mut data = Vec::with_capacity(cached.data.len());
    for (position, slot) in cached.data.into_iter().enumerate() {
        let item = match slot {
            Some(item) => item,
            None => fresh_embedding(&mut fresh_items, position)?,
        };
        data.push(item);
    }
    let micros = end_time
        .signed_duration_since(start_time)
        .num_microseconds()
        .ok_or(MergeError::ResponseTime)?;
    let mut hidden_params = cached.hidden_params;
    hidden_params.insert("cache_hit".to_owned(), Json::Bool(true));
    Ok(Some(MergedEmbeddingResponse {
        model: cached.model,
        data,
        usage,
        hidden_params,
        response_headers: cached
            .response_headers
            .filter(|headers| !headers.is_empty()),
        response_ms: Decimal::new(micros, 3),
    }))
}

// One fresh item fills each empty cached position, in order, and takes that position as its index
fn fresh_embedding(
    fresh_items: &mut Iter<'_, Embedding>,
    position: usize,
) -> Result<Embedding, MergeError> {
    let next = fresh_items
        .next()
        .ok_or(MergeError::MissingFresh { position })?;
    let index =
        i64::try_from(position).map_err(|_conversion| MergeError::Overflow { field: "index" })?;
    Ok(Embedding {
        embedding: next.embedding.clone(),
        index,
    })
}
