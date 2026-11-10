use http::Method;

use super::QdrantSemanticCache;
use super::error::QdrantSemanticError;
use super::ports::QdrantRequest;
use super::wire::{Point, PointPayload, PointsRequest};
use crate::arguments::json::Json;

pub(crate) async fn write_point(
    cache: &QdrantSemanticCache,
    key: &str,
    prompt: String,
    vector: Vec<f64>,
    value: &Json,
) -> Result<(), QdrantSemanticError> {
    let payload = PointPayload {
        litellm_cache_key: key.to_owned(),
        text: prompt,
        response: serde_json::to_string(value).map_err(QdrantSemanticError::Encode)?,
    };
    let body = PointsRequest {
        points: vec![Point {
            id: cache.uuid.next_id().to_string(),
            vector,
            payload,
        }],
    };
    cache
        .http_client
        .send(QdrantRequest {
            method: Method::PUT,
            path: format!("/collections/{}/points", cache.collection_name),
            query: vec![("wait".to_owned(), "true".to_owned())],
            body: Some(serde_json::to_vec(&body).map_err(QdrantSemanticError::Encode)?),
        })
        .await?;
    Ok(())
}
