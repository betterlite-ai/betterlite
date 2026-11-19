pub mod cache_cost_scenario;
pub mod price_cache_tokens;
pub mod pricing_validation;
pub mod prompt_cache_prediction;
pub mod required_prices;
pub mod service;
pub mod token_buckets;
pub mod valid_price;

use betterlite_models::price_lookup::PriceLookup;
use jiff::tz::TimeZoneDatabase;

#[path = "prompt_cache_scenarios/tests/cache_token_buckets.rs"]
mod cache_token_buckets;
#[path = "prompt_cache_scenarios/tests/support.rs"]
mod test_support;

// The two price sources a cache scenario reads, so no pricing step reaches for a global catalog.
pub struct PriceEnvironment<'environment> {
    pub lookup: &'environment (dyn PriceLookup + Send + Sync),
    pub database: &'environment TimeZoneDatabase,
}
