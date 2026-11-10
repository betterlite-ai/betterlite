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
    let path = format!("/collections/{}/points", cache.collection_name);
    let reply = cache
        .http_client
        .send(QdrantRequest {
            method: Method::PUT,
            path: path.clone(),
            query: vec![("wait".to_owned(), "true".to_owned())],
            body: Some(serde_json::to_vec(&body).map_err(QdrantSemanticError::Encode)?),
        })
        .await?;
    if (200..300).contains(&reply.status) {
        return Ok(());
    }
    Err(QdrantSemanticError::PointWriteRejected {
        path,
        status: reply.status,
        body: String::from_utf8(reply.body).unwrap_or_default(),
    })
}
