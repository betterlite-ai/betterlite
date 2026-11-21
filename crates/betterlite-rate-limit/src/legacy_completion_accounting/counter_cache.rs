use std::fmt;

use async_trait::async_trait;
use betterlite_auth::resolved_key::auth::ParentOtelSpan;

use crate::rate_limiters::legacy_rate_limit_counters::usage_cache::{
    LegacyCacheValue, MinuteCounter, UsageCacheError,
};

use super::error::AccountingError;

// the key both admission and these hooks move by one per request, kept in the local cache only
pub const GLOBAL_PARALLEL_KEY: &str = "global_max_parallel_requests";

// InternalUsageCache's single-key calls; the handler's batch cache has no local_only read or increment
#[async_trait]
pub trait CounterCache: Send + Sync {
    // internal_usage_cache.async_get_cache
    async fn get(
        &self,
        key: &str,
        parent_otel_span: Option<&ParentOtelSpan>,
        local_only: bool,
    ) -> Result<Option<LegacyCacheValue>, UsageCacheError>;

    // internal_usage_cache.async_set_cache
    async fn set(
        &self,
        key: &str,
        value: LegacyCacheValue,
        ttl: u64,
        parent_otel_span: Option<&ParentOtelSpan>,
    ) -> Result<(), UsageCacheError>;

    // internal_usage_cache.async_increment_cache
    async fn increment(
        &self,
        key: &str,
        delta: i64,
        parent_otel_span: Option<&ParentOtelSpan>,
        local_only: bool,
    ) -> Result<(), UsageCacheError>;
}

// a plain read of one minute counter; a bare number there is LiteLLM's TypeError on current["current_requests"]
pub(super) async fn read_counter(
    counters: &dyn CounterCache,
    key: &str,
    parent_otel_span: Option<&ParentOtelSpan>,
) -> Result<Option<MinuteCounter>, AccountingError> {
    let stored = counters
        .get(key, parent_otel_span, false)
        .await
        .map_err(|source| AccountingError::Cache {
            key: key.to_owned(),
            source,
        })?;
    match stored {
        None => Ok(None),
        Some(LegacyCacheValue::Counter(counter)) => Ok(Some(counter)),
        Some(LegacyCacheValue::Count(_)) => Err(AccountingError::NotACounter {
            key: key.to_owned(),
        }),
    }
}

// max(current_requests - 1, 0): a finished request gives its slot back, never below zero
pub(super) fn released_requests(current_requests: i64, key: &str) -> Result<i64, AccountingError> {
    let lowered = current_requests
        .checked_sub(1)
        .ok_or_else(|| AccountingError::Overflow {
            key: key.to_owned(),
        })?;
    Ok(lowered.max(0))
}

// the new value the success and failure hooks print, in the field order of LiteLLM's dict
impl fmt::Display for MinuteCounter {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{{current_requests: {}, current_tpm: {}, current_rpm: {}}}",
            self.current_requests, self.current_tpm, self.current_rpm
        )
    }
}
