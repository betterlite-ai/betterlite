use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use betterlite_cache::handler::retrieve::CacheSource;
use betterlite_cache::types::cache_type::LiteLlmCacheType;
use betterlite_cache::types::call_type::CachingSupportedCallType;
use betterlite_logging::logging_success::ports::SuccessClock;
use betterlite_logging::verbose_hooks::{ConsoleSink, LoggerSink};
use betterlite_logging::worker::LoggingWorker;
use betterlite_models::resolution::ResolutionContext;

use crate::caching::hit_callbacks::job::CacheHitSuccessJob;

use super::error::TokenCountError;

// LiteLLM reads time.perf_counter() twice around the cache read; betterlite injects the counter so the duration is testable
pub type PerfClock = Arc<dyn Fn() -> Duration + Send + Sync>;

// B196's response-token count, which a cached embedding without prompt_tokens needs
#[async_trait]
pub trait ResponseTokens: Sync {
    async fn count_response_tokens(&self, text: &str) -> Result<i64, TokenCountError>;
}

// Everything _async_get_cache and _sync_get_cache read from the gateway besides the request and the logging object
pub struct LookupServices<'svc, L: LoggerSink, C: ConsoleSink> {
    pub cache: Option<&'svc CacheSource<'svc, L, C>>,
    pub cache_type: LiteLlmCacheType,
    pub supported_call_types: Option<&'svc [CachingSupportedCallType]>,
    pub resolution: &'svc ResolutionContext<'svc>,
    pub worker: &'svc LoggingWorker<CacheHitSuccessJob>,
    pub tokens: &'svc dyn ResponseTokens,
    pub perf: PerfClock,
    pub wall: Arc<dyn SuccessClock>,
}
