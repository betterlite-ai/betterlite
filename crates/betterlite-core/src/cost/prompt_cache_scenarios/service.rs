use betterlite_anthropic::cache_aware_routing::services::CacheCostService;
use betterlite_anthropic::cache_aware_routing::services::MinimumCacheTokens;
use betterlite_anthropic::cache_aware_routing::services::PriceRequest;
use betterlite_anthropic::cache_aware_routing::services::ScenarioRequest;
use betterlite_anthropic::cache_aware_routing::types::CacheCostScenario;
use betterlite_anthropic::cache_aware_routing::types::CacheTokenBuckets;
use betterlite_models::catalog::ModelCatalog;
use chrono::DateTime;
use chrono::Utc;
use rust_decimal::Decimal;

use crate::cost::prompt_cache_scenarios::PriceEnvironment;
use crate::cost::prompt_cache_scenarios::cache_cost_scenario::cache_cost_scenario;
use crate::cost::prompt_cache_scenarios::price_cache_tokens::price_cache_tokens;
use crate::cost::prompt_cache_scenarios::token_buckets::token_buckets;
use crate::message_utilities::prompt_cache_checks::PromptCacheSettings;
use crate::message_utilities::prompt_cache_checks::get_prompt_cache_min_tokens;

pub struct CatalogCacheCostService<'environment> {
    pub environment: PriceEnvironment<'environment>,
}

impl CacheCostService for CatalogCacheCostService<'_> {
    fn buckets(
        &self,
        prefix_tokens: i64,
        suffix_tokens: i64,
        read_tokens: i64,
        ttl_seconds: u32,
    ) -> CacheTokenBuckets {
        token_buckets(prefix_tokens, suffix_tokens, read_tokens, ttl_seconds)
    }

    fn scenario(
        &self,
        request: &ScenarioRequest<'_>,
        moment: DateTime<Utc>,
    ) -> Option<CacheCostScenario> {
        cache_cost_scenario(&self.environment, moment, request)
    }

    fn price(&self, request: &PriceRequest<'_>, moment: DateTime<Utc>) -> Option<Decimal> {
        price_cache_tokens(&self.environment, moment, request)
    }
}

pub struct PromptCacheMinimum<'environment> {
    pub settings: PromptCacheSettings,
    pub catalog: &'environment ModelCatalog,
}

impl MinimumCacheTokens for PromptCacheMinimum<'_> {
    fn minimum_tokens(&self, model: &str) -> i64 {
        let minimum = get_prompt_cache_min_tokens(model, self.settings, self.catalog);
        i64::try_from(minimum).unwrap_or(i64::MAX)
    }
}

mod tests;
