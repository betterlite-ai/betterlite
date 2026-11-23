use async_trait::async_trait;
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
