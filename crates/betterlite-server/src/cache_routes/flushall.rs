use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse as _, Response};

use super::CacheRoutesState;
use super::admin::blocking;
use super::wire::{SuccessBody, unhealthy};

pub async fn cache_flushall(State(state): State<CacheRoutesState>) -> Response {
    let Some(cache) = state.cache.current() else {
        return unhealthy("Cache not initialized. litellm.cache is None");
    };
    let kind = cache.cache_type();
    let Some(redis) = cache.redis_admin() else {
        return unhealthy(&format!("Cache type {kind} does not support flushing"));
    };
    match blocking(redis, |admin| admin.flushall()).await {
        Ok(()) => (StatusCode::OK, Json(SuccessBody { status: "success" })).into_response(),
        Err(source) => unhealthy(&source.to_string()),
    }
}

mod tests;
