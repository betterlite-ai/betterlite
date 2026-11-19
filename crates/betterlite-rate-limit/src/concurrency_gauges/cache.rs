use std::collections::BTreeMap;
use std::time::Duration;

use crate::limiter_records::CacheCounterValue;

#[derive(Clone, Debug, PartialEq)]
pub enum GaugeCacheValue {
    Registry(BTreeMap<String, f64>),
    Counter(CacheCounterValue),
}

// Every gauge read and write asks for the local tier only, the local_only=True LiteLLM passes on these paths
#[async_trait::async_trait]
pub trait GaugeCache: Send + Sync {
    async fn get(&self, key: &str, local_only: bool) -> Option<GaugeCacheValue>;

    async fn set(&self, key: &str, value: GaugeCacheValue, ttl: Duration, local_only: bool);

    async fn batch_get(
        &self,
        keys: &[String],
        local_only: bool,
    ) -> Option<Vec<Option<GaugeCacheValue>>>;
}
