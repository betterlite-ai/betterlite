use serde_json::Number;

use super::error::QdrantSemanticError;
use super::wire::SearchHit;
use crate::arguments::json::{Json, Object};
use crate::semantic_cache_records::decode_cached_response;

pub const SIMILARITY_FIELD: &str = "semantic-similarity";

pub(crate) fn read_hit(
    hit: Option<&SearchHit>,
    key: &str,
    threshold: f64,
    metadata: Option<&mut Object>,
) -> Result<Option<Json>, QdrantSemanticError> {
    let Some(found) = hit else {
        set_zero_similarity(metadata);
        return Ok(None);
    };
    if found.payload.litellm_cache_key.as_deref() != Some(key) {
        tracing::debug!("Qdrant semantic-cache hit did not match cache key scope");
        set_zero_similarity(metadata);
        return Ok(None);
    }
    if found.payload.text.is_none() {
        return Err(QdrantSemanticError::MissingPayloadField { field: "text" });
    }
    write_similarity(metadata, found.score)?;
    if found.score < threshold {
        return Ok(None);
    }
    let Some(response) = found.payload.response.as_deref() else {
        return Err(QdrantSemanticError::MissingPayloadField { field: "response" });
    };
    Ok(decode_cached_response(Some(response.as_bytes()))?)
}

fn write_similarity(
    metadata: Option<&mut Object>,
    similarity: f64,
) -> Result<(), QdrantSemanticError> {
    let Some(target) = metadata else {
        return Ok(());
    };
    let Some(number) = Number::from_f64(similarity) else {
        return Err(QdrantSemanticError::NonFiniteSimilarity { similarity });
    };
    target.insert(SIMILARITY_FIELD.to_owned(), Json::Number(number));
    Ok(())
}

pub(crate) fn set_zero_similarity(metadata: Option<&mut Object>) {
    let Some(target) = metadata else {
        return;
    };
    let Some(number) = Number::from_f64(0.0_f64) else {
        return;
    };
    target.insert(SIMILARITY_FIELD.to_owned(), Json::Number(number));
}
