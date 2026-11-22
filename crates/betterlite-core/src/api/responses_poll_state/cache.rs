use std::sync::Arc;

use async_trait::async_trait;
use betterlite_redis::RedisCache;
use betterlite_redis::cache_primitives::namespace::check_and_fix_namespace;
use num_traits::ToPrimitive as _;
use redis::cmd;
use rust_decimal::Decimal;
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PollingCacheError {
    #[error("the polling cache could not be read: {message}")]
    Read { message: String },
    #[error("the polling cache could not be written: {message}")]
    Write { message: String },
    #[error("the polling cache could not be deleted: {message}")]
    Delete { message: String },
}

// get and update need the stored bytes and their errors; B126's async_get_cache turns both into None
#[async_trait]
pub trait PollingCache: Send + Sync {
    async fn read(&self, key: &str) -> Result<Option<Vec<u8>>, PollingCacheError>;

    async fn write(&self, key: &str, value: &str, ttl: i64) -> Result<(), PollingCacheError>;

    async fn delete(&self, key: &str) -> Result<i64, PollingCacheError>;
}

pub struct RedisPollingCache {
    cache: Arc<RedisCache>,
}

impl RedisPollingCache {
    #[must_use]
    pub const fn new(cache: Arc<RedisCache>) -> Self {
        Self { cache }
    }

    fn namespaced(&self, key: &str) -> String {
        check_and_fix_namespace(self.cache.namespace.as_deref(), Some(key))
            .unwrap_or_else(|| key.to_owned())
    }
}

#[async_trait]
impl PollingCache for RedisPollingCache {
    async fn read(&self, key: &str) -> Result<Option<Vec<u8>>, PollingCacheError> {
        let namespaced = self.namespaced(key);
        let client = self
            .cache
            .transport
            .async_client()
            .await
            .map_err(|source| PollingCacheError::Read {
                message: source.to_string(),
            })?;
        let mut command = cmd("GET");
        command.arg(namespaced);
        client
            .query::<Option<Vec<u8>>>(&command)
            .await
            .map_err(|source| PollingCacheError::Read {
                message: source.to_string(),
            })
    }

    async fn write(&self, key: &str, value: &str, ttl: i64) -> Result<(), PollingCacheError> {
        let seconds = Decimal::from(ttl).to_f64();
        let _stored = self
            .cache
            .values
            .async_set_cache(Some(key), value, seconds, false)
            .await
            .map_err(|source| PollingCacheError::Write {
                message: source.to_string(),
            })?;
        Ok(())
    }

    async fn delete(&self, key: &str) -> Result<i64, PollingCacheError> {
        self.cache
            .admin
            .async_delete_cache(key)
            .await
            .map_err(|source| PollingCacheError::Delete {
                message: source.to_string(),
            })
    }
}
