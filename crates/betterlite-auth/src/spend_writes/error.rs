use betterlite_cache::backend::BackendError;
use betterlite_redis::redis_cache::error::RedisCacheError;
use betterlite_redis::redis_cache_admin::RedisAdminError;
use betterlite_redis::redis_client_reuse::error::ClientReuseError;
use betterlite_redis::redis_counters::RedisCountersError;
use betterlite_redis::redis_live_client::LiveClientError;
use thiserror::Error;

use crate::spend_counter_batch::batch::SpendCounterBatchError;
use crate::spend_reseed::cache::dual::ReseedCacheError;
use crate::spend_reseed::reseed::CoalescedError;
use crate::spend_reseed::values::SpendValueError;

// the Redis and in-memory operations only betterlite's spend counter writes call
#[derive(Debug, Error)]
pub enum LayersError {
    #[error(transparent)]
    Redis(#[from] RedisCacheError),
    #[error(transparent)]
    Admin(#[from] RedisAdminError),
    #[error(transparent)]
    Memory(#[from] BackendError),
    #[error("redis returned {0}, which has no decimal counter form")]
    NonFinite(f64),
    #[error("the spend cache has no Redis tier")]
    Unavailable,
}

impl LayersError {
    // _apply_spend_counter_increments lets a breaker open or a timeout pass, since both clear on their own
    #[must_use]
    pub fn is_transient(&self) -> bool {
        match self {
            Self::Redis(error) => redis_transient(error),
            Self::Admin(_) | Self::Memory(_) | Self::NonFinite(_) | Self::Unavailable => false,
        }
    }
}

#[derive(Debug, Error)]
pub enum SpendWriteError {
    #[error(transparent)]
    Batch(#[from] SpendCounterBatchError),
    #[error(transparent)]
    Cache(#[from] ReseedCacheError),
    #[error(transparent)]
    Layers(#[from] LayersError),
    #[error(transparent)]
    Reseed(#[from] CoalescedError<ReseedCacheError>),
    #[error(transparent)]
    Value(#[from] SpendValueError),
}

impl SpendWriteError {
    #[must_use]
    pub fn is_transient(&self) -> bool {
        match self {
            Self::Layers(error) => error.is_transient(),
            Self::Cache(error) => cache_transient(error),
            Self::Batch(_) | Self::Reseed(_) | Self::Value(_) => false,
        }
    }
}

fn cache_transient(error: &ReseedCacheError) -> bool {
    match error {
        ReseedCacheError::Redis(error) => redis_transient(error),
        ReseedCacheError::Values(_)
        | ReseedCacheError::Memory(_)
        | ReseedCacheError::OutOfRange(_)
        | ReseedCacheError::Unavailable => false,
    }
}

fn redis_transient(error: &RedisCacheError) -> bool {
    match error {
        RedisCacheError::Breaker(_) => true,
        RedisCacheError::ClientReuse(error) => reuse_transient(error),
        RedisCacheError::LiveClient(error) => live_transient(error),
        RedisCacheError::Counters(error) => counters_transient(error),
        RedisCacheError::Settings(_)
        | RedisCacheError::Factory(_)
        | RedisCacheError::Decode(_)
        | RedisCacheError::Join(_) => false,
    }
}

fn counters_transient(error: &RedisCountersError) -> bool {
    match error {
        RedisCountersError::Client(error) => live_transient(error),
        RedisCountersError::Reuse(error) => reuse_transient(error),
        RedisCountersError::Breaker(_) => true,
        RedisCountersError::Decode(_)
        | RedisCountersError::UnexpectedReply { .. }
        | RedisCountersError::InvalidTtl
        | RedisCountersError::NonFiniteValue => false,
    }
}

fn reuse_transient(error: &ClientReuseError) -> bool {
    match error {
        ClientReuseError::Timeout(_) => true,
        ClientReuseError::Live(error) => live_transient(error),
        ClientReuseError::Factory(_)
        | ClientReuseError::NoPubSubNode
        | ClientReuseError::ClusterPubSub => false,
    }
}

// redis-py reports a pool wait timeout as a connection error chained from a timeout, so a Redis io timeout counts
fn live_transient(error: &LiveClientError) -> bool {
    match error {
        LiveClientError::Redis(error) => error.is_timeout(),
        LiveClientError::InvalidSetting { .. }
        | LiveClientError::CredentialSourceMissing
        | LiveClientError::Credential(_)
        | LiveClientError::TlsFile { .. }
        | LiveClientError::NoSentinelMaster { .. }
        | LiveClientError::Join(_) => false,
    }
}
