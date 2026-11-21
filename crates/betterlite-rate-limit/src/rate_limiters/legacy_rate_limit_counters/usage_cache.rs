use async_trait::async_trait;
use betterlite_auth::resolved_key::auth::ParentOtelSpan;
use betterlite_cache::dual_cache::DualCacheError;
use serde::{Deserialize, Serialize};

// the counter object a minute key holds: `{id}::{minute}::request_count`
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct MinuteCounter {
    pub current_requests: i64,
    pub current_tpm: i64,
    pub current_rpm: i64,
}

// what a legacy cache key holds: a minute counter, or a bare whole number such as the global slot count
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(untagged)]
pub enum LegacyCacheValue {
    Count(i64),
    Counter(MinuteCounter),
}

// InternalUsageCache's failure, carried up so the caller logs the DualCache transport's own message
#[derive(Debug, thiserror::Error)]
pub enum UsageCacheError {
    #[error("the usage cache call failed: {0}")]
    Transport(#[source] DualCacheError),
}

// InternalUsageCache as the legacy limiter calls it; an implementation owns its failure handling, as DualCache does
#[async_trait]
pub trait LegacyUsageCache: Send + Sync {
    // internal_usage_cache.async_batch_get_cache: None is a failed read, as DualCache answers it
    async fn batch_get(
        &self,
        keys: &[Option<String>],
        parent_otel_span: Option<&ParentOtelSpan>,
    ) -> Result<Option<Vec<Option<LegacyCacheValue>>>, UsageCacheError>;

    // internal_usage_cache.async_batch_set_cache
    async fn batch_set(
        &self,
        entries: &[(String, LegacyCacheValue)],
        ttl: u64,
        parent_otel_span: Option<&ParentOtelSpan>,
        local_only: bool,
    ) -> Result<(), UsageCacheError>;
}
