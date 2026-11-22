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
    #[error("the polling cache could not be read: {0}")]
    Read(String),
    #[error("the polling cache could not be written: {0}")]
    Write(String),
    #[error("the polling cache could not be deleted: {0}")]
    Delete(String),
}

// get and update need the stored bytes and their errors; B126's async_get_cache turns both into None
#[async_trait]
pub trait PollingCache: Send + Sync {
    async fn read(&self, key: &str) -> Result<Option<Vec<u8>>, PollingCacheError>;

    async fn write(&self, key: &str, value: &str, ttl: i64) -> Result<(), PollingCacheError>;

    async fn delete(&self, key: &str) -> Result<i64, PollingCacheError>;
}

pub struct RedisPollingCache {
    pub cache: Arc<RedisCache>,
}

#[async_trait]
impl PollingCache for RedisPollingCache {
    async fn read(&self, key: &str) -> Result<Option<Vec<u8>>, PollingCacheError> {
        let namespaced = check_and_fix_namespace(self.cache.namespace.as_deref(), Some(key))
            .unwrap_or_else(|| key.to_owned());
        let mut command = cmd("GET");
        command.arg(namespaced);
        self.cache
            .transport
            .async_client()
            .await
            .map_err(|source| PollingCacheError::Read(source.to_string()))?
            .query::<Option<Vec<u8>>>(&command)
            .await
            .map_err(|source| PollingCacheError::Read(source.to_string()))
    }

    async fn write(&self, key: &str, value: &str, ttl: i64) -> Result<(), PollingCacheError> {
        let seconds = Decimal::from(ttl).to_f64();
        let _stored = self
            .cache
            .values
            .async_set_cache(Some(key), value, seconds, false)
            .await
            .map_err(|source| PollingCacheError::Write(source.to_string()))?;
        Ok(())
    }

    async fn delete(&self, key: &str) -> Result<i64, PollingCacheError> {
        self.cache
            .admin
            .async_delete_cache(key)
            .await
            .map_err(|source| PollingCacheError::Delete(source.to_string()))
    }
}
