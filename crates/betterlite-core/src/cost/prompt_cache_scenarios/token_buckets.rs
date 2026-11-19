use betterlite_anthropic::cache_aware_routing::types::CacheTokenBuckets;
use betterlite_anthropic::cache_aware_routing::types::FIVE_MINUTE_TTL_SECONDS;
use betterlite_anthropic::cache_aware_routing::types::ONE_HOUR_TTL_SECONDS;

#[must_use]
pub fn token_buckets(
    prefix_tokens: i64,
    suffix_tokens: i64,
    read_tokens: i64,
    ttl_seconds: u32,
) -> CacheTokenBuckets {
    let creation = prefix_tokens.saturating_sub(read_tokens).max(0);
    CacheTokenBuckets {
        uncached_input_tokens: suffix_tokens,
        cache_read_input_tokens: read_tokens,
        cache_creation_5m_input_tokens: if ttl_seconds == FIVE_MINUTE_TTL_SECONDS {
            creation
        } else {
            0
        },
        cache_creation_1h_input_tokens: if ttl_seconds == ONE_HOUR_TTL_SECONDS {
            creation
        } else {
            0
        },
    }
}

mod tests;
