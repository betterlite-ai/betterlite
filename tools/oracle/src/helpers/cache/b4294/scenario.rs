use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_cache::backend::memory::MemoryCacheOps;
use betterlite_cache::backend::memory::local::LocalMemoryOps;
use betterlite_cache::backend::{BackendError, CacheResult};
use betterlite_cache::batch_reads::{BatchReadOptions, BatchReads};
use betterlite_cache::dual_cache::{CacheOptions, DualCache, DualCacheSettings, Ttl};
use betterlite_context::local_ttl_cache::CacheValue;
use betterlite_models::decisions::Json;
use betterlite_redis::redis_cache::RedisCache;
use serde_json::json;

use super::input::{CacheFields, Call, Client, Entry, Fields};
use super::server::{Allow, FakeRedis, Rejecting, connect_cache};
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::HelperResult;

pub(super) const LOCAL: CacheOptions = CacheOptions {
    ttl: None,
    local_only: true,
    refresh_ttl: false,
};

pub(super) const THROTTLED: BatchReadOptions = BatchReadOptions {
    local_only: false,
    throttle_redis: true,
};

pub(super) const EVERY: BatchReadOptions = BatchReadOptions {
    local_only: false,
    throttle_redis: false,
};

// the memory tier's own batch read, either answering or failing the way B2226's tier can
pub(super) struct HarnessMemory {
    inner: LocalMemoryOps,
    failures: Failures,
}

#[derive(Clone, Copy, Default)]
pub(super) struct Failures {
    pub(super) reads: bool,
    pub(super) writes: bool,
}

impl MemoryCacheOps for HarnessMemory {
    fn default_ttl(&self) -> i64 {
        self.inner.default_ttl()
    }

    fn get_cache(&self, key: &str) -> CacheResult<Option<CacheValue>> {
        if self.failures.reads {
            return Err(BackendError::Unsupported("the memory read failed"));
        }
        self.inner.get_cache(key)
    }

    fn set_cache(&self, key: &str, value: CacheValue, ttl: Option<f64>) -> CacheResult<()> {
        if self.failures.writes {
            return Err(BackendError::Unsupported("the memory write failed"));
        }
        self.inner.set_cache(key, value, ttl)
    }

    fn delete_cache(&self, key: &str) -> CacheResult<()> {
        self.inner.delete_cache(key)
    }

    fn flush_cache(&self) -> CacheResult<()> {
        self.inner.flush_cache()
    }

    fn batch_get_cache(&self, keys: &[String]) -> CacheResult<Vec<Option<CacheValue>>> {
        if self.failures.reads {
            return Err(BackendError::Unsupported("the memory read failed"));
        }
        self.inner.batch_get_cache(keys)
    }

    fn increment_cache(&self, key: &str, value: f64, ttl: Option<f64>) -> CacheResult<f64> {
        self.inner.increment_cache(key, value, ttl)
    }

    fn ttl_entries(&self) -> CacheResult<Vec<(String, f64)>> {
        self.inner.ttl_entries()
    }
}

pub(super) struct Setup {
    pub(super) shared_server: FakeRedis,
    pub(super) own_server: FakeRedis,
    _rejecting_server: FakeRedis,
    pub(super) shared: Arc<RedisCache>,
    pub(super) own: Arc<RedisCache>,
    pub(super) rejecting: Arc<RedisCache>,
}

impl Setup {
    pub(super) async fn start(call: &Fields) -> Result<Self, Box<HelperError>> {
        let shared_server = FakeRedis::start().await?;
        let own_server = FakeRedis::start().await?;
        let rejecting_server = FakeRedis::start().await?;
        for reply in &call.replies {
            shared_server.script_mget(reply)?;
        }
        for reply in &call.own_replies {
            own_server.script_mget(reply)?;
        }
        let shared = connect_cache(shared_server.addr, Arc::new(Allow)).await?;
        let own = connect_cache(own_server.addr, Arc::new(Allow)).await?;
        let rejecting = connect_cache(rejecting_server.addr, Arc::new(Rejecting)).await?;
        Ok(Self {
            shared_server,
            own_server,
            _rejecting_server: rejecting_server,
            shared,
            own,
            rejecting,
        })
    }
}

fn settings(call: &Fields) -> DualCacheSettings {
    let default_in_memory_ttl = call.default_in_memory_ttl;
    DualCacheSettings {
        default_in_memory_ttl,
        ..DualCacheSettings::from_environment(&BTreeMap::<String, String>::new())
    }
}

pub(super) fn build_cache(
    tier: Option<&Arc<RedisCache>>,
    call: &Fields,
    failures: Failures,
) -> Result<DualCache<HarnessMemory>, Box<HelperError>> {
    let clock = call.clock;
    let now = Arc::new(move || clock);
    let inner = LocalMemoryOps::new(1024, now).or_value_error()?;
    let memory = HarnessMemory { inner, failures };
    Ok(DualCache::new(
        Arc::new(memory),
        tier.cloned(),
        settings(call),
    ))
}

// a cache with the case's memory entries seeded, the way every scenario starts its read
pub(super) fn seeded(
    call: &Fields,
    tier: Option<&Arc<RedisCache>>,
) -> Result<DualCache<HarnessMemory>, Box<HelperError>> {
    let cache = build_cache(tier, call, Failures::default())?;
    seed(&cache, &call.memory);
    Ok(cache)
}

pub(super) fn seed(cache: &DualCache<HarnessMemory>, entries: &[Entry]) {
    for entry in entries {
        cache.set_cache(&entry.key, &entry.value, LOCAL);
    }
}

pub(super) fn reservation(
    reserved: (Vec<String>, BTreeMap<String, Option<f64>>),
) -> serde_json::Value {
    let (due, previous) = reserved;
    json!({"due": due, "previous": previous})
}

pub(super) async fn ttl_values(
    cache: &DualCache<HarnessMemory>,
    keys: &[String],
) -> Result<Vec<Option<Ttl>>, Box<HelperError>> {
    let mut values = Vec::with_capacity(keys.len());
    for key in keys {
        values.push(cache.async_get_ttl(key).await.or_value_error()?);
    }
    Ok(values)
}

pub(super) async fn group(
    setup: &Setup,
    entries: &[CacheFields],
    call: &Fields,
) -> Result<
    (
        Vec<DualCache<HarnessMemory>>,
        Vec<Option<Vec<Option<Json>>>>,
    ),
    Box<HelperError>,
> {
    let mut caches = Vec::with_capacity(entries.len());
    for entry in entries {
        let failures = Failures {
            reads: entry.fail_reads,
            writes: entry.fail_writes,
        };
        let chosen = match entry.client {
            Client::Shared => Some(&setup.shared),
            Client::Own => Some(&setup.own),
            Client::Rejecting => Some(&setup.rejecting),
            Client::None => None,
        };
        let built = build_cache(chosen, call, failures)?;
        seed(&built, &entry.memory);
        caches.push(built);
    }
    let reads: Vec<(&DualCache<HarnessMemory>, &[String])> = caches
        .iter()
        .zip(entries)
        .map(|(built, entry)| (built, entry.keys.as_slice()))
        .collect();
    let results =
        <DualCache<HarnessMemory> as BatchReads>::async_batch_get_cache_shared(&reads).await;
    Ok((caches, results))
}

pub(crate) async fn run(call: &Call) -> HelperResult {
    match call {
        Call::ReserveThrottleAndRollback(fields) => super::scenarios::reserve(fields).await,
        Call::SyncReadBackfill(fields) => super::scenarios::sync_backfill(fields).await,
        Call::SyncReadErrors(fields) => super::scenarios::sync_errors(fields).await,
        Call::AsyncReadThrottleSwitch(fields) => super::scenarios::async_switch(fields).await,
        Call::SharedReadMerge(fields) => super::scenarios::shared_merge(fields).await,
        Call::SharedReadFailures(fields) => super::scenarios::shared_failures(fields).await,
    }
}
