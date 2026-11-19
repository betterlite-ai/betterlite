use betterlite_logging::cache_observer::ObservationCache;
use betterlite_models::resolution::ResolutionContext;
use chrono::DateTime;
use chrono::Utc;
use rust_decimal::Decimal;

use crate::cache_aware_routing::types::CacheCostScenario;
use crate::cache_aware_routing::types::CacheTokenBuckets;
use crate::credentials::AnthropicSettings;

pub struct ScenarioRequest<'request> {
    pub model: &'request str,
    pub deployment_id: &'request str,
    pub tokens: &'request CacheTokenBuckets,
}

pub struct PriceRequest<'request> {
    pub model: &'request str,
    pub deployment_id: &'request str,
    pub tokens: &'request CacheTokenBuckets,
    pub completion_tokens: i64,
}

// Ports _buckets, _scenario and price_cache_tokens: the price catalog lives in betterlite-core, so the host passes it in.
pub trait CacheCostService: Send + Sync {
    fn buckets(
        &self,
        prefix_tokens: i64,
        suffix_tokens: i64,
        read_tokens: i64,
        ttl_seconds: u32,
    ) -> CacheTokenBuckets;

    fn scenario(
        &self,
        request: &ScenarioRequest<'_>,
        moment: DateTime<Utc>,
    ) -> Option<CacheCostScenario>;

    fn price(&self, request: &PriceRequest<'_>, moment: DateTime<Utc>) -> Option<Decimal>;
}

pub trait MinimumCacheTokens: Send + Sync {
    fn minimum_tokens(&self, model: &str) -> i64;
}

// Every outside value prediction reads, so no step of it reaches for a global catalog, clock or cache.
pub struct PredictionServices<'services> {
    pub settings: &'services AnthropicSettings,
    pub resolution: &'services ResolutionContext<'services>,
    pub cache: &'services dyn ObservationCache,
    pub cost: &'services dyn CacheCostService,
    pub minimum: &'services dyn MinimumCacheTokens,
    pub billing_time: DateTime<Utc>,
}
