use std::num::TryFromIntError;
use std::sync::Arc;

use async_trait::async_trait;
use betterlite_cache::configuration::cache::CacheLifecycleError;
use betterlite_cache::types::ping::HealthCheckCacheParams;
use betterlite_redact::error::RedactionError;
use betterlite_redis::redis_cache_admin::RedisAdminError;
use betterlite_redis::redis_cache_admin::info_parse::InfoValue;
use indexmap::IndexMap;
use thiserror::Error;
use tokio::task::{JoinError, JoinSet};

// LiteLLM's litellm.cache read through one port, so the routes can be tested without a live Redis.
#[async_trait]
pub trait CacheAdmin: Send + Sync {
    fn cache_type(&self) -> String;
    fn masked_cache_params(&self) -> String;
    fn health_check_params(&self) -> HealthCheckCacheParams;
    async fn ping(&self) -> Result<bool, CacheAdminError>;
    async fn add_test_key(&self) -> Result<(), CacheAdminError>;
    async fn delete_cache_keys(&self, keys: &[String]) -> Result<(), CacheAdminError>;
    fn redis_admin(&self) -> Option<Arc<dyn RedisAdmin>>;
}

// The blocking Redis administration calls, run on the blocking pool by the routes.
pub trait RedisAdmin: Send + Sync {
    fn client_list(&self) -> Result<Vec<IndexMap<String, String>>, CacheAdminError>;
    fn info(&self) -> Result<IndexMap<String, InfoValue>, CacheAdminError>;
    fn flushall(&self) -> Result<(), CacheAdminError>;
}

#[derive(Debug, Error)]
pub enum CacheAdminError {
    #[error("the cache ping failed")]
    Ping {
        #[source]
        source: CacheLifecycleError,
    },
    #[error("deleting the cache keys failed")]
    Delete {
        #[source]
        source: CacheLifecycleError,
    },
    #[error("reading the redis client list failed")]
    ClientList {
        #[source]
        source: RedisAdminError,
    },
    #[error("reading the redis info failed")]
    Info {
        #[source]
        source: RedisAdminError,
    },
    #[error("flushing the cache failed")]
    Flush {
        #[source]
        source: RedisAdminError,
    },
    #[error("a blocking cache call did not finish")]
    Blocking {
        #[source]
        source: JoinError,
    },
    #[error("a blocking cache call never ran")]
    BlockingMissing,
    #[error("the cache test write could not be prepared")]
    TestWrite {
        #[source]
        source: RedactionError,
    },
    #[error("the cache redis flush size {size} does not fit the cache envelope")]
    FlushSize {
        size: usize,
        #[source]
        source: TryFromIntError,
    },
}

pub(crate) async fn blocking<T, F>(
    admin: Arc<dyn RedisAdmin>,
    call: F,
) -> Result<T, CacheAdminError>
where
    T: Send + 'static,
    F: FnOnce(&dyn RedisAdmin) -> Result<T, CacheAdminError> + Send + 'static,
{
    let mut tasks = JoinSet::new();
    tasks.spawn_blocking(move || call(admin.as_ref()));
    match tasks.join_next().await {
        Some(Ok(value)) => value,
        Some(Err(source)) => Err(CacheAdminError::Blocking { source }),
        None => Err(CacheAdminError::BlockingMissing),
    }
}
