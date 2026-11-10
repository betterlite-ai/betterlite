use http::Method;
use serde_json::Number;

use super::error::QdrantSemanticError;
use super::options::CACHE_KEY_FIELD_NAME;
use super::ports::QdrantRequest;
use super::wire::{
    CacheKeyCondition, CacheKeyFilter, MatchValue, QuantizationRescore, SearchHit, SearchParams,
    SearchRequest, SearchResponse,
};
use super::{QdrantSemanticCache, decode};
use crate::arguments::json::{Json, Object};
use crate::semantic_cache_records::decode_cached_response;

pub const SIMILARITY_FIELD: &str = "semantic-similarity";

pub(crate) async fn search(
    cache: &QdrantSemanticCache,
    key: &str,
    vector: Vec<f64>,
    metadata: Option<&mut Object>,
) -> Result<Option<Json>, QdrantSemanticError> {
    let body = SearchRequest {
        vector,
        params: SearchParams {
            quantization: QuantizationRescore {
                ignore: false,
                rescore: true,
                oversampling: 3.0,
            },
        },
        limit: 1,
        with_payload: true,
        filter: CacheKeyFilter {
            must: vec![CacheKeyCondition {
                key: CACHE_KEY_FIELD_NAME,
                matches: MatchValue {
                    value: key.to_owned(),
                },
            }],
        },
    };
    let reply = cache
        .transport
        .send(QdrantRequest {
            method: Method::POST,
            path: format!("/collections/{}/points/search", cache.collection_name),
            query: Vec::new(),
            body: Some(serde_json::to_vec(&body).map_err(QdrantSemanticError::Encode)?),
        })
        .await?;
    let response: SearchResponse = decode(&reply.body, "points search")?;
    let hit = response.result.as_deref().and_then(|hits| hits.first());
    read_hit(hit, key, cache.similarity_threshold, metadata)
}

fn read_hit(
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
