use std::sync::Arc;

use crate::limiter_records::LimiterClock;

use super::cache::SaturationCache;
use super::settings::PriorityPolicy;

mod tests;

// _PROXY_MaxParallelRequestsHandler_v3 as the dynamic handler builds it: the usage cache, the clock a check runs at, and the window its descriptors count in
pub struct V3Limiter {
    pub internal_usage_cache: Arc<dyn SaturationCache>,
    pub time_provider: Option<Arc<dyn LimiterClock>>,
    pub window_size: i64,
}

// _PROXY_DynamicRateLimitHandlerV3: the state a saturation-aware hook carries between requests
pub struct DynamicRateLimitHandler<Router> {
    pub internal_usage_cache: Arc<dyn SaturationCache>,
    pub v3_limiter: V3Limiter,
    pub policy: PriorityPolicy,
    pub llm_router: Option<Arc<Router>>,
}

impl<Router> DynamicRateLimitHandler<Router> {
    // __init__: the handler and the v3 limiter share one usage cache, and the time provider reaches the limiter as given
    #[must_use]
    pub fn new(
        internal_usage_cache: Arc<dyn SaturationCache>,
        time_provider: Option<Arc<dyn LimiterClock>>,
        policy: PriorityPolicy,
        window_size: i64,
    ) -> Self {
        Self {
            v3_limiter: V3Limiter {
                internal_usage_cache: Arc::clone(&internal_usage_cache),
                time_provider,
                window_size,
            },
            internal_usage_cache,
            policy,
            llm_router: None,
        }
    }

    // update_variables: LiteLLM stores the router so a hook can read a model group's capacity mid-request
    // ast-grep-ignore: no-one-line-functions - the proxy calls update_variables by name
    pub fn update_variables(&mut self, llm_router: Arc<Router>) {
        self.llm_router = Some(llm_router);
    }
}
