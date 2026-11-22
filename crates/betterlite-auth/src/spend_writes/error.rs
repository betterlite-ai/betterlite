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
            Self::Cache(ReseedCacheError::Redis(error)) => redis_transient(error),
            Self::Cache(_) | Self::Batch(_) | Self::Reseed(_) | Self::Value(_) => false,
        }
    }
}

// LiteLLM's is_redis_timeout_failure walks the chain, so a timed-out Redis io error counts under any wrapper, and an open breaker counts too
fn redis_transient(error: &RedisCacheError) -> bool {
    match error {
        RedisCacheError::Breaker(_)
        | RedisCacheError::ClientReuse(ClientReuseError::Timeout(_))
        | RedisCacheError::Counters(
            RedisCountersError::Breaker(_)
            | RedisCountersError::Reuse(ClientReuseError::Timeout(_)),
        ) => true,
        RedisCacheError::ClientReuse(ClientReuseError::Live(LiveClientError::Redis(error)))
        | RedisCacheError::LiveClient(LiveClientError::Redis(error))
        | RedisCacheError::Counters(
            RedisCountersError::Client(LiveClientError::Redis(error))
            | RedisCountersError::Reuse(ClientReuseError::Live(LiveClientError::Redis(error))),
        ) => error.is_timeout(),
        RedisCacheError::ClientReuse(_)
        | RedisCacheError::Counters(_)
        | RedisCacheError::LiveClient(_)
        | RedisCacheError::Settings(_)
        | RedisCacheError::Factory(_)
        | RedisCacheError::Decode(_)
        | RedisCacheError::Join(_) => false,
    }
}
