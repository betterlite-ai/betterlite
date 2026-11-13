pub mod local;

use async_trait::async_trait;
use betterlite_context::local_ttl_cache::CacheValue;

use super::{CacheBackend, CacheResult, Ttl};
use crate::types::pipeline::RedisPipelineIncrement;

pub trait MemoryCacheOps: Send + Sync {
    fn default_ttl(&self) -> i64;
    fn get_cache(&self, key: &str) -> CacheResult<Option<CacheValue>>;
    fn set_cache(&self, key: &str, value: CacheValue, ttl: Ttl) -> CacheResult<()>;
    fn delete_cache(&self, key: &str) -> CacheResult<()>;
    fn flush_cache(&self) -> CacheResult<()>;
    fn batch_get_cache(&self, keys: &[String]) -> CacheResult<Vec<Option<CacheValue>>>;
    fn increment_cache(&self, key: &str, value: f64, ttl: Ttl) -> CacheResult<f64>;
    // every stored absolute expiry, in a stable order that the sort by expiry keeps for equal expiries
    fn ttl_entries(&self) -> CacheResult<Vec<(String, f64)>>;
}

pub struct MemoryCache<Ops: MemoryCacheOps> {
    pub operations: Ops,
}

#[async_trait]
impl<Ops: MemoryCacheOps> CacheBackend for MemoryCache<Ops> {
    fn default_ttl(&self) -> i64 {
        self.operations.default_ttl()
    }

    fn get_cache(&self, key: &str) -> CacheResult<Option<CacheValue>> {
        self.operations.get_cache(key)
    }

    fn set_cache(&self, key: &str, value: CacheValue, ttl: Ttl) -> CacheResult<()> {
        self.operations.set_cache(key, value, ttl)
    }

    fn delete_cache(&self, key: &str) -> CacheResult<()> {
        self.operations.delete_cache(key)
    }

    fn flush_cache(&self) -> CacheResult<()> {
        self.operations.flush_cache()
    }

    async fn async_get_cache(&self, key: &str) -> CacheResult<Option<CacheValue>> {
        self.operations.get_cache(key)
    }

    async fn async_set_cache(&self, key: &str, value: CacheValue, ttl: Ttl) -> CacheResult<()> {
        self.operations.set_cache(key, value, ttl)
    }

    async fn async_set_cache_pipeline(
        &self,
        entries: Vec<(String, CacheValue)>,
        ttl: Ttl,
    ) -> CacheResult<()> {
        for (key, value) in entries {
            self.operations.set_cache(&key, value, ttl)?;
        }
        Ok(())
    }

    async fn async_batch_get_cache(&self, keys: &[String]) -> CacheResult<Vec<Option<CacheValue>>> {
        self.operations.batch_get_cache(keys)
    }

    async fn async_increment(&self, key: &str, value: f64, ttl: Ttl) -> CacheResult<f64> {
        self.operations.increment_cache(key, value, ttl)
    }

    async fn async_increment_pipeline(
        &self,
        increments: Vec<RedisPipelineIncrement>,
    ) -> CacheResult<Vec<f64>> {
        let mut results = Vec::with_capacity(increments.len());
        for increment in increments {
            // the pipeline forwards its own kwargs, never the record ttl, which stays unused
            results.push(self.operations.increment_cache(
                &increment.key,
                increment.increment_value,
                None,
            )?);
        }
        Ok(results)
    }

    async fn async_get_ttl(&self, key: &str) -> CacheResult<Option<f64>> {
        let entries = self.operations.ttl_entries()?;
        Ok(entries
            .into_iter()
            .find(|(name, _)| name.as_str() == key)
            .map(|(_, expiry)| expiry))
    }

    async fn async_get_oldest_n_keys(&self, limit: usize) -> CacheResult<Vec<String>> {
        let mut entries = self.operations.ttl_entries()?;
        entries.sort_by(|left, right| left.1.total_cmp(&right.1));
        Ok(entries
            .into_iter()
            .take(limit)
            .map(|(key, _)| key)
            .collect())
    }

    async fn disconnect(&self) -> CacheResult<()> {
        Ok(())
    }
}
