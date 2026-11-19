use std::collections::BTreeMap;

use betterlite_context::local_ttl_cache::CacheValue;
use betterlite_models::decisions::Json;

use super::db::{PrefetchDatabase, PrefetchDatabaseError};
use super::entries::{AuthObjectRefs, PrefetchEntry};
use super::memory::{MemoryWriteError, PrefetchCache, PrefetchMemory};
use super::redis_rows::{PrefetchRedis, RedisRowsError};
use super::rows::{RowKind, RowObject, validate_row};
use crate::auth_cache::payload::PayloadError;

#[derive(Debug, thiserror::Error)]
#[error("the Redis result is not a JSON object")]
pub struct RowMapError;

#[derive(Debug, thiserror::Error)]
pub enum PrefetchError {
    #[error("the auth prefetch Redis read failed")]
    Redis(#[source] RedisRowsError),
    #[error(transparent)]
    RowMap(#[from] RowMapError),
    #[error(transparent)]
    Payload(#[from] PayloadError),
    #[error(transparent)]
    Memory(#[from] MemoryWriteError),
    #[error(transparent)]
    Database(#[from] PrefetchDatabaseError),
    #[error("the auth prefetch value has no in-memory representation")]
    Value(#[source] serde_json::Error),
}

// _RowValues.validate_python: only a string-keyed mapping is accepted, and null values are misses
#[must_use]
pub fn row_map(value: Json) -> Result<BTreeMap<String, Json>, RowMapError> {
    let Json::Object(entries) = value else {
        return Err(RowMapError);
    };
    Ok(entries
        .into_iter()
        .filter(|(_key, entry)| !matches!(entry, &Json::Null))
        .collect())
}

#[must_use]
pub fn missing_in_memory(
    entries: &[PrefetchEntry],
    memory: &dyn PrefetchMemory,
) -> Vec<PrefetchEntry> {
    entries
        .iter()
        .filter(|entry| memory.get(&entry.key).is_none())
        .cloned()
        .collect()
}

// a None ttl reaches the memory tier without one, which then applies its own default
pub fn set_in_memory(
    memory: &dyn PrefetchMemory,
    key: &str,
    value: &Json,
    ttl: Option<f64>,
) -> Result<(), PrefetchError> {
    let wire = serde_json::to_value(value).map_err(PrefetchError::Value)?;
    let value: CacheValue = serde_json::from_value(wire).map_err(PrefetchError::Value)?;
    memory.set(key, value, ttl).map_err(PrefetchError::Memory)
}

// one MGET of the sorted keys, duplicates kept; a failed pipeline reads as no rows
pub async fn read_redis_values(
    keys: &[String],
    redis: &dyn PrefetchRedis,
) -> Result<BTreeMap<String, Json>, PrefetchError> {
    let mut sorted = keys.to_vec();
    sorted.sort();
    match redis.read(&sorted).await {
        Ok(value) => row_map(value).map_err(PrefetchError::RowMap),
        Err(RedisRowsError::Batch { message }) => {
            tracing::debug!(
                %message,
                "auth prefetch Redis read failed, filling from the database"
            );
            Ok(BTreeMap::new())
        }
        Err(error) => Err(PrefetchError::Redis(error)),
    }
}

// Redis rows are cached as found: the typed getters validate them when they read
pub async fn fill_from_redis(
    entries: &[PrefetchEntry],
    redis: &dyn PrefetchRedis,
    memory: &dyn PrefetchMemory,
) -> Result<(), PrefetchError> {
    if entries.is_empty() {
        return Ok(());
    }
    let keys: Vec<String> = entries.iter().map(|entry| entry.key.clone()).collect();
    let found = read_redis_values(&keys, redis).await?;
    for entry in entries {
        if let Some(value) = found.get(&entry.key) {
            set_in_memory(memory, &entry.key, value, entry.ttl)?;
        }
    }
    Ok(())
}

// serialization of every entry finishes before the first memory write, and writes keep entry order
pub async fn write_back(
    writes: &[(PrefetchEntry, RowObject)],
    cache: &dyn PrefetchCache,
    redis: Option<&dyn PrefetchRedis>,
) -> Result<(), PrefetchError> {
    let mut payloads = Vec::with_capacity(writes.len());
    for (entry, object) in writes {
        payloads.push((entry.key.clone(), object.payload()?, entry.ttl));
    }
    let memory = cache.ordinary_memory();
    let memory_default = cache.default_memory_ttl();
    for (key, payload, ttl) in &payloads {
        set_in_memory(memory, key, payload, ttl.or(memory_default))?;
    }
    let Some(redis) = redis else {
        return Ok(());
    };
    if redis.has_batch() {
        for (key, payload, ttl) in &payloads {
            redis.queue_set(key, payload, *ttl)?;
        }
        return Ok(());
    }
    redis
        .set_pipeline(&payloads)
        .await
        .map_err(PrefetchError::Redis)
}

// the lanes one database fill reads and writes through, with the one clock reading it takes
pub struct FillTargets<'target> {
    pub cache: &'target dyn PrefetchCache,
    pub redis: Option<&'target dyn PrefetchRedis>,
    pub database: &'target dyn PrefetchDatabase,
    pub clock: &'target dyn PrefetchClock,
}

// one SELECT for every still-missing kind, one clock reading after it, one validation per kind
pub async fn fill_from_db(
    refs: &AuthObjectRefs,
    entries: &[PrefetchEntry],
    targets: &FillTargets<'_>,
) -> Result<(), PrefetchError> {
    if entries.is_empty() {
        return Ok(());
    }
    let mut kinds: Vec<RowKind> = Vec::new();
    for entry in entries {
        if !kinds.contains(&entry.row) {
            kinds.push(entry.row);
        }
    }
    kinds.sort();
    let rows = targets.database.fetch_rows(refs, &kinds).await?;
    let refreshed_at = targets.clock.now();
    let mut objects: BTreeMap<RowKind, Option<RowObject>> = BTreeMap::new();
    for kind in &kinds {
        objects.insert(*kind, validate_row(*kind, rows.get(kind), refreshed_at));
    }
    let writes: Vec<(PrefetchEntry, RowObject)> = entries
        .iter()
        .filter_map(|entry| {
            objects
                .get(&entry.row)
                .and_then(Option::as_ref)
                .map(|object| (entry.clone(), object.clone()))
        })
        .collect();
    if writes.is_empty() {
        return Ok(());
    }
    write_back(&writes, targets.cache, targets.redis).await
}

// time.time() at the point LiteLLM reads it: once, after the query
pub trait PrefetchClock: Send + Sync {
    fn now(&self) -> f64;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SystemPrefetchClock;

impl PrefetchClock for SystemPrefetchClock {
    fn now(&self) -> f64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_secs_f64())
            .unwrap_or_default()
    }
}
