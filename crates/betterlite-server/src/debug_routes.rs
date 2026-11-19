pub mod cache_counts;
pub mod cache_items;
pub mod cache_memory;
pub mod otel_spans;
pub mod schema;

mod asyncio_tasks;
mod gc_configure;
mod memory_details;
mod memory_summary;
mod memory_usage;
mod query;
mod unsupported;

#[cfg(test)]
pub(crate) mod test_support;

use std::sync::Arc;

use axum::Router;
use axum::routing::{get, post};
use betterlite_config::loader::EnvironmentLookup;

use cache_memory::CacheSource;
use otel_spans::FinishedSpanSource;

const PROFILE_VARIABLE: &str = "LITELLM_PROFILE";

#[derive(Clone)]
pub struct DebugRoutesState {
    pub user_api_key_cache: Arc<dyn CacheSource>,
    pub user_key_object_cache: Arc<dyn CacheSource>,
    pub llm_router_cache: Option<Arc<dyn CacheSource>>,
    pub proxy_logging_cache: Arc<dyn CacheSource>,
    pub spans: Option<Arc<dyn FinishedSpanSource>>,
}

pub fn router(state: DebugRoutesState, environment: &dyn EnvironmentLookup) -> Router {
    let mut router = Router::new()
        .route("/debug/asyncio-tasks", get(asyncio_tasks::handler))
        .route("/memory-usage-in-mem-cache", get(cache_counts::handler))
        .route(
            "/memory-usage-in-mem-cache-items",
            get(cache_items::handler),
        )
        .route("/debug/memory/summary", get(memory_summary::handler))
        .route("/debug/memory/details", get(memory_details::handler))
        .route("/debug/memory/gc/configure", post(gc_configure::handler))
        .route("/otel-spans", get(otel_spans::handler));
    if profiling_enabled(environment) {
        router = router.route("/memory-usage", get(memory_usage::handler));
    }
    router.with_state(state)
}

// LiteLLM registers the profiling route at import time, and the mount happens once at startup
fn profiling_enabled(environment: &dyn EnvironmentLookup) -> bool {
    environment.lookup(PROFILE_VARIABLE).is_some_and(|value| {
        // ast-grep-ignore: no-python-habits - betterlite_env::flag also accepts 1, yes and on, and LiteLLM does not
        value.eq_ignore_ascii_case("true")
    })
}
