use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use betterlite_auth::auth_cache::AuthCache;
use betterlite_auth::auth_cache::WriteOptions;
use betterlite_auth::auth_cache::WriteTtl;
use betterlite_auth::auth_cache::keys::{Partition, partition};
use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_cache::backend::memory::MemoryCacheOps;
use betterlite_cache::batch_reads::BatchReadOptions;
use betterlite_cache::batch_reads::BatchReads as _;
use betterlite_cache::batch_reads::DeclaredBatchRead;
use betterlite_cache::dual_cache::DualCache;
use betterlite_cache::dual_cache::MemoryMaxSize;
use betterlite_models::decisions::Json;
use betterlite_redis::redis_cache::RedisCache;
use betterlite_redis::request_batch::batches::RequestRedisBatches;

use super::SpendCacheUpdateError;

// one armed read per partition, since LiteLLM's single cache is two tiers here
pub struct ArmedSpendRead {
    pub keys: Vec<String>,
    pub reads: Vec<DeclaredBatchRead>,
}

#[async_trait]
pub trait SpendCache: Send + Sync {
    fn management_ttl(&self) -> f64;

    fn declare(
        &self,
        keys: &[String],
        batches: &RequestRedisBatches,
    ) -> Result<Option<ArmedSpendRead>, SpendCacheUpdateError>;

    async fn read(
        &self,
        keys: &[String],
        local_only: bool,
    ) -> Result<Vec<Option<Json>>, SpendCacheUpdateError>;

    async fn resolve(
        &self,
        armed: ArmedSpendRead,
    ) -> Result<Vec<Option<Json>>, SpendCacheUpdateError>;

    async fn auth(&self, key: &str) -> Result<Option<UserAPIKeyAuth>, SpendCacheUpdateError>;

    async fn write(&self, entries: &[(String, Json)], ttl: f64, local_only: bool);
}

#[async_trait]
impl<Memory> SpendCache for AuthCache<DualCache<Memory>>
where
    Memory: MemoryCacheOps + MemoryMaxSize + 'static,
{
    fn management_ttl(&self) -> f64 {
        self.management_object_ttl()
    }

    fn declare(
        &self,
        keys: &[String],
        batches: &RequestRedisBatches,
    ) -> Result<Option<ArmedSpendRead>, SpendCacheUpdateError> {
        let mut targets = Vec::new();
        for group in partition_groups(keys) {
            let Some(first) = group.first() else {
                continue;
            };
            let Some(redis) = tier_redis(self.tier(first)) else {
                return Ok(None);
            };
            targets.push((self.tier(first), redis, group));
        }
        let mut reads = Vec::new();
        for (tier, redis, group) in targets {
            let batch = batches.batch(&redis);
            let read = tier
                .declare_batch_get(&group, &batch)
                .map_err(SpendCacheUpdateError::Declare)?;
            reads.push(read);
        }
        Ok(Some(ArmedSpendRead {
            keys: keys.to_vec(),
            reads,
        }))
    }

    async fn read(
        &self,
        keys: &[String],
        local_only: bool,
    ) -> Result<Vec<Option<Json>>, SpendCacheUpdateError> {
        let mut found: BTreeMap<String, Option<Json>> = BTreeMap::new();
        for group in partition_groups(keys) {
            let Some(first) = group.first() else {
                continue;
            };
            let options = BatchReadOptions {
                local_only,
                throttle_redis: false,
            };
            let Some(values) = self
                .tier(first)
                .async_batch_get_cache(&group, options)
                .await
            else {
                return Err(SpendCacheUpdateError::CacheRead);
            };
            found.extend(group.into_iter().zip(values));
        }
        Ok(keys.iter().map(|key| found.remove(key).flatten()).collect())
    }

    async fn resolve(
        &self,
        armed: ArmedSpendRead,
    ) -> Result<Vec<Option<Json>>, SpendCacheUpdateError> {
        let mut found: BTreeMap<String, Option<Json>> = BTreeMap::new();
        for read in armed.reads {
            let read_keys = read.keys.clone();
            let Some(first) = read_keys.first() else {
                continue;
            };
            let resolved = self
                .tier(first)
                .async_resolve_batch_get(read)
                .await
                .map_err(SpendCacheUpdateError::Resolve)?;
            found.extend(read_keys.into_iter().zip(resolved));
        }
        Ok(armed
            .keys
            .iter()
            .map(|key| found.remove(key).flatten())
            .collect())
    }

    async fn auth(&self, key: &str) -> Result<Option<UserAPIKeyAuth>, SpendCacheUpdateError> {
        self.async_get_auth(key, false)
            .await
            .map_err(SpendCacheUpdateError::KeyAuth)
    }

    async fn write(&self, entries: &[(String, Json)], ttl: f64, local_only: bool) {
        let options = WriteOptions {
            ttl: Some(WriteTtl { seconds: ttl }),
            local_only,
            refresh_ttl: false,
        };
        if let Err(error) = self.async_set_pipeline(entries, options).await {
            tracing::error!(
                "Spend tracking - failed to write the updated spend objects to cache: {error}"
            );
        }
    }
}

fn partition_groups(keys: &[String]) -> Vec<Vec<String>> {
    let mut ordinary = Vec::new();
    let mut key_objects = Vec::new();
    for key in keys {
        match partition(key) {
            Partition::Ordinary => ordinary.push(key.clone()),
            Partition::KeyObject => key_objects.push(key.clone()),
        }
    }
    vec![ordinary, key_objects]
}

fn tier_redis<Memory>(tier: &DualCache<Memory>) -> Option<Arc<RedisCache>>
where
    Memory: MemoryCacheOps,
{
    match tier.redis_cache.read() {
        Ok(guard) => guard.clone(),
        Err(poisoned) => poisoned.into_inner().clone(),
    }
}
