use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use betterlite_redis::batch::error::BatchError;
use betterlite_redis::cache_primitives::protocol::RedisScalar;
use betterlite_redis::redis_counters::RedisCountersError;
use tokio::sync::Mutex;

use self::counter_value::CounterValueError;
use crate::limiter_records::{CacheCounterValue, CounterReplyError, ScriptReply};

pub mod batch_counter_read_script;
pub mod batch_get;
pub mod batch_rate_limiter_script;
pub mod cluster_detect;
pub mod counter_value;
pub mod hash_tag_group;
pub mod in_memory_window;
pub mod no_retry;
pub mod pipeline_scripts;
pub mod read_fallback;
pub mod redis_read_order;
pub mod slot_compute;

mod tests;

pub type CounterValues = Vec<Option<CacheCounterValue>>;

pub type ScriptCall = (Vec<String>, Vec<RedisScalar>);

pub type ScriptHandle = Pin<Box<dyn Future<Output = Result<ScriptReply, Arc<BatchError>>> + Send>>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RedisCacheKind {
    Standalone,
    Cluster,
}

#[async_trait::async_trait]
pub trait CounterCache: Send + Sync {
    async fn get_cache(&self, key: &str, local_only: bool) -> Option<CacheCounterValue>;

    async fn set_cache(&self, key: &str, value: CacheCounterValue, ttl: i64, local_only: bool);

    async fn batch_get_cache(&self, keys: &[String], local_only: bool) -> Option<CounterValues>;

    fn redis_cache_kind(&self) -> Option<RedisCacheKind>;
}

#[async_trait::async_trait]
pub trait CounterScript: Send + Sync {
    fn source(&self) -> &str;

    async fn run(
        &self,
        keys: &[String],
        args: &[RedisScalar],
    ) -> Result<ScriptReply, RedisCountersError>;
}

pub trait ScriptBatch: Send + Sync {
    fn script(
        &self,
        script: Arc<dyn CounterScript>,
        keys: &[String],
        args: &[RedisScalar],
    ) -> ScriptHandle;
}

pub trait RequestBatchSource: Send + Sync {
    fn active_batch(&self) -> Option<Arc<dyn ScriptBatch>>;
}

pub struct WindowStore {
    pub cache: Arc<dyn CounterCache>,
    pub rate_limiter_script: Option<Arc<dyn CounterScript>>,
    pub counter_read_script: Option<Arc<dyn CounterScript>>,
    pub request_batch: Option<Arc<dyn RequestBatchSource>>,
    // LiteLLM's _check_and_increment_lock, held across the reads and writes of one sliding-window pass
    pub check_and_increment_lock: Mutex<()>,
}

#[derive(Debug, thiserror::Error)]
pub enum WindowStoreError {
    #[error("the Redis rate-limit script run failed")]
    Script(#[from] RedisCountersError),
    #[error("the rate-limit script reply could not be read")]
    Reply(#[from] CounterReplyError),
    #[error("the {name} script is not registered")]
    MissingScript { name: &'static str },
    #[error("the window keys are not window/counter pairs: {count} keys")]
    UnpairedKeys { count: usize },
    #[error("the cached window start is not a whole number")]
    WindowStart(#[from] CounterValueError),
    #[error("the counter is already at the largest whole number")]
    CounterOverflow,
}
