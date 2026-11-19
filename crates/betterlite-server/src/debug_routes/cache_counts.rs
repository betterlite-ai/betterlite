use axum::Json;
use axum::extract::{Extension, State};
use axum::response::{IntoResponse as _, Response};
use serde::Serialize;

use crate::auth::auth_identity::AuthContext;
use crate::debug_routes::DebugRoutesState;
use crate::debug_routes::cache_memory::{CacheSnapshot, CacheSource as _};
use crate::tracing_routes::router::unauthenticated;

mod tests;

#[derive(Serialize)]
struct CacheCountsBody {
    num_items_in_user_api_key_cache: usize,
    num_items_in_llm_router_cache: usize,
    num_items_in_proxy_logging_obj_cache: usize,
}

fn item_count(snapshot: &CacheSnapshot) -> usize {
    snapshot
        .cache_dict
        .len()
        .saturating_add(snapshot.ttl_dict.len())
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
    let user_items = item_count(&ordinary).saturating_add(item_count(&key_objects));
    let router_items = match state.llm_router_cache.as_ref() {
        Some(source) => item_count(&source.snapshot()),
        None => 0,
    };
    let logging_items = item_count(&state.proxy_logging_cache.snapshot());
    Json(CacheCountsBody {
        num_items_in_user_api_key_cache: user_items,
        num_items_in_llm_router_cache: router_items,
        num_items_in_proxy_logging_obj_cache: logging_items,
    })
    .into_response()
}
