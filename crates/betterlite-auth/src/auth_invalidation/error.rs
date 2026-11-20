use std::sync::Arc;

use betterlite_cache::backend::BackendError;
use betterlite_cache::dual_cache::DualCacheError;
use betterlite_redis::batch::error::BatchError;
use betterlite_redis::redis_cache_values::error::RedisCacheValuesError;
use rust_decimal::Decimal;
use thiserror::Error;

use crate::auth_cache::AuthCacheError;
use crate::auth_cache::payload::PayloadError;

#[derive(Debug, Error)]
pub enum InvalidationError {
    #[error("the cached value could not be serialized")]
    Payload(#[source] Box<PayloadError>),
    #[error("the auth cache write failed")]
    Write(#[source] Box<AuthCacheError>),
    #[error("the auth cache operation failed")]
    Backend(#[source] Box<DualCacheError>),
    #[error("the queued auth cache delete failed")]
    QueuedDelete(#[source] Arc<BatchError>),
    #[error("the cache entry could not be held in memory")]
    Memory(#[source] Box<BackendError>),
    #[error("the spend {0} has no finite cache form")]
    SpendOutOfRange(Decimal),
    #[error("the cache timestamp {0} has no float form")]
    Timestamp(Decimal),
    #[error(
        "Spend was reset in the database, but Redis is unreachable and still holds the pre-reset counter. Retry once Redis is reachable."
    )]
    SpendResetUnavailable(#[source] Box<RedisCacheValuesError>),
}
