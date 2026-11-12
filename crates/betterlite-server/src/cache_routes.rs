pub mod admin;
pub mod delete;
pub mod flushall;
pub mod live;
pub mod masking;
pub mod ping;
pub mod ping_params;
pub mod redis_info;
pub mod test_write;
pub mod wire;

use std::sync::Arc;

use axum::Router;
use axum::routing::{get, post};

use live::LiveResponseCache;

#[path = "cache_routes/tests/auth.rs"]
mod auth;
#[path = "cache_routes/tests/redis_support.rs"]
mod redis_support;
#[path = "cache_routes/tests/support.rs"]
mod support_tests;

#[derive(Clone)]
pub struct CacheRoutesState {
    pub cache: Arc<dyn LiveResponseCache>,
}

pub fn router(state: CacheRoutesState) -> Router {
    Router::new()
        .route("/cache/ping", get(ping::cache_ping))
        .route("/cache/delete", post(delete::cache_delete))
        .route("/cache/redis/info", get(redis_info::cache_redis_info))
        .route("/cache/flushall", post(flushall::cache_flushall))
        .with_state(state)
}
