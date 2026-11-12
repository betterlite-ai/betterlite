use axum::Json;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse as _, Response};
use serde::Deserialize;

use super::CacheRoutesState;
use super::wire::{SuccessBody, delete_failed};

#[derive(Deserialize)]
struct DeleteRequest {
    #[serde(default)]
    keys: Option<Vec<String>>,
}

pub async fn cache_delete(State(state): State<CacheRoutesState>, body: Bytes) -> Response {
    let Some(cache) = state.cache.current() else {
        return delete_failed("Cache not initialized. litellm.cache is None");
    };
    let request: DeleteRequest = match serde_json::from_slice(&body) {
        Ok(request) => request,
        Err(source) => return delete_failed(&source.to_string()),
    };
    let kind = cache.cache_type();
    if kind != "redis" {
        return delete_failed(&format!(
            "Cache type {kind} does not support deleting a key. only `redis` is supported"
        ));
    }
    let Some(keys) = request.keys else {
        return delete_failed("keys is required");
    };
    match cache.delete_cache_keys(&keys).await {
        Ok(()) => (StatusCode::OK, Json(SuccessBody { status: "success" })).into_response(),
        Err(source) => delete_failed(&source.to_string()),
    }
}

mod tests;
