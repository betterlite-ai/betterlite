use betterlite_anthropic::cache_aware_routing::types::CacheTokenBuckets;
use betterlite_models::price_lookup::PriceRecord;
use betterlite_models::tiered_pricing::Scalar;

use crate::cost::prompt_cache_scenarios::valid_price::valid_price;

// Ports _has_required_prices: a zero price is present, a missing or unusable one is not.
#[must_use]
pub fn has_required_prices(
    prices: &PriceRecord,
    tokens: &CacheTokenBuckets,
    completion_tokens: i64,
) -> bool {
    let required = [
        ("input_cost_per_token", true),
        (
            "cache_read_input_token_cost",
            tokens.cache_read_input_tokens > 0,
        ),
        (
            "cache_creation_input_token_cost",
            tokens.cache_creation_5m_input_tokens > 0,
        ),
        (
            "cache_creation_input_token_cost_above_1hr",
            tokens.cache_creation_1h_input_tokens > 0,
        ),
        ("output_cost_per_token", completion_tokens > 0),
    ];
    if required
        .iter()
        .any(|(key, needed)| *needed && !valid_price(prices.get(*key)))
    {
        return false;
    }
    prices.iter().all(|(key, value)| {
        let related = required
            .iter()
            .any(|(base, needed)| *needed && above_variant(key, base));
        !related || matches!(value, Scalar::Null) || valid_price(Some(value))
    })
}

fn above_variant(key: &str, base: &str) -> bool {
    key.strip_prefix(base)
        .is_some_and(|rest| rest.starts_with("_above_"))
}

mod tests;
