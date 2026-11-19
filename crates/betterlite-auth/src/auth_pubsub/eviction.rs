use betterlite_cache::backend::BackendError;
use thiserror::Error;

use super::message::Invalidation;

#[derive(Debug, Error)]
pub enum CacheError {
    #[error("the local cache entry {cache_key} could not be dropped")]
    Eviction {
        cache_key: String,
        #[source]
        source: BackendError,
    },
    #[error("the local cache entry {cache_key} could not be updated")]
    Failed {
        cache_key: String,
        #[source]
        source: BackendError,
    },
}

pub trait LocalEviction: Send + Sync {
    fn apply(&self, invalidation: Invalidation) -> Result<(), CacheError>;
}
