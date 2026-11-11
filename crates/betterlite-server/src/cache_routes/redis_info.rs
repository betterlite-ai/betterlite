use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse as _, Response};
use betterlite_redis::redis_cache_admin::info_parse::InfoValue;
use indexmap::IndexMap;
use serde::Serialize;

use super::CacheRoutesState;
use super::admin::blocking;
use super::wire::unhealthy;

#[path = "redis_info/tests/client_list_restricted.rs"]
mod client_list_restricted;
mod tests;

const CLIENT_LIST_UNAVAILABLE: &str = "CLIENT LIST command not available on this Redis instance";

// LiteLLM writes -1 when CLIENT LIST is restricted and the client list length otherwise.
#[derive(Serialize)]
#[serde(untagged)]
enum ClientCount {
    Listed(usize),
    Unavailable(isize),
}

#[derive(Serialize)]
struct RedisInfoResponse {
    num_clients: ClientCount,
    clients: RedisClients,
    info: IndexMap<String, InfoValue>,
}

// LiteLLM returns the parsed client list, or one warning string when CLIENT LIST is restricted.
#[derive(Serialize)]
#[serde(untagged)]
enum RedisClients {
    Listed(Vec<IndexMap<String, String>>),
    Unavailable([&'static str; 1]),
}

pub async fn cache_redis_info(State(state): State<CacheRoutesState>) -> Response {
    let Some(cache) = state.cache.current() else {
        return unhealthy("Cache not initialized. litellm.cache is None");
    };
    let kind = cache.cache_type();
    let Some(redis) = cache.redis_admin() else {
        return unhealthy(&format!("Cache type {kind} does not support redis info"));
    };
    let (num_clients, clients) =
        match blocking(Arc::clone(&redis), |admin| admin.client_list()).await {
            Ok(clients) => (
                ClientCount::Listed(clients.len()),
                RedisClients::Listed(clients),
            ),
            Err(error) => {
                tracing::warn!(
                    "CLIENT LIST command failed (likely restricted on managed Redis): {error}"
                );
                (
                    ClientCount::Unavailable(-1),
                    RedisClients::Unavailable([CLIENT_LIST_UNAVAILABLE]),
                )
            }
        };
    let info = match blocking(redis, |admin| admin.info()).await {
        Ok(info) => info,
        Err(error) => return unhealthy(&error.to_string()),
    };
    let body = RedisInfoResponse {
        num_clients,
        clients,
        info,
    };
    (StatusCode::OK, Json(body)).into_response()
}
