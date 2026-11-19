use std::collections::BTreeMap;

use axum::Json;
use axum::extract::{Extension, State};
use axum::response::{IntoResponse as _, Response};
use betterlite_models::decisions::Json as CacheJson;
use serde::Serialize;

use crate::auth::auth_identity::AuthContext;
use crate::debug_routes::DebugRoutesState;
use crate::debug_routes::cache_memory::{CacheSnapshot, CacheSource as _};
use crate::tracing_routes::router::unauthenticated;

mod tests;

#[derive(Serialize)]
struct CacheItemsBody {
    user_api_key_cache: BTreeMap<String, CacheJson>,
    user_api_key_ttl: BTreeMap<String, f64>,
    user_key_object_cache: BTreeMap<String, CacheJson>,
    user_key_object_ttl: BTreeMap<String, f64>,
    llm_router_cache: BTreeMap<String, CacheJson>,
    llm_router_ttl: BTreeMap<String, f64>,
    proxy_logging_obj_cache: BTreeMap<String, CacheJson>,
    proxy_logging_obj_ttl: BTreeMap<String, f64>,
}

pub(super) async fn handler(
    State(state): State<DebugRoutesState>,
    caller: Option<Extension<AuthContext>>,
) -> Response {
    if caller.is_none() {
        return unauthenticated();
    }
    let ordinary = state.user_api_key_cache.snapshot();
    let key_objects = state.user_key_object_cache.snapshot();
    let router = match state.llm_router_cache.as_ref() {
        Some(source) => source.snapshot(),
        None => CacheSnapshot::default(),
    };
    let logging = state.proxy_logging_cache.snapshot();
    Json(CacheItemsBody {
        user_api_key_cache: ordinary.cache_dict,
        user_api_key_ttl: ordinary.ttl_dict,
        user_key_object_cache: key_objects.cache_dict,
        user_key_object_ttl: key_objects.ttl_dict,
        llm_router_cache: router.cache_dict,
        llm_router_ttl: router.ttl_dict,
        proxy_logging_obj_cache: logging.cache_dict,
        proxy_logging_obj_ttl: logging.ttl_dict,
    })
    .into_response()
}
