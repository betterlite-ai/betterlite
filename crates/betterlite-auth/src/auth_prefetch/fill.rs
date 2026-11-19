use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use betterlite_context::local_ttl_cache::CacheValue;
use betterlite_db::key_user_writes::hash::hash_token;
use betterlite_models::decisions::Json;

use super::db::{PrefetchDatabase, PrefetchDatabaseError};
use super::entries::{AuthObjectRefs, PrefetchEntry};
use super::memory::{MemoryWriteError, PrefetchCache, PrefetchMemory};
use super::redis_rows::{PrefetchRedis, RedisRowsError};
use super::rows::{RowKind, RowObject, validate_row};
use crate::auth_cache::keys::CacheKey;
use crate::auth_cache::payload::{self, PayloadError};

#[derive(Debug, thiserror::Error)]
pub enum PrefetchError {
    #[error("the auth prefetch Redis operation failed: {0}")]
    Redis(#[from] RedisRowsError),
    #[error("the Redis result is not a JSON object")]
    Shape,
    #[error(transparent)]
    Payload(#[from] PayloadError),
    #[error(transparent)]
    Memory(#[from] MemoryWriteError),
    #[error(transparent)]
    Database(#[from] PrefetchDatabaseError),
    #[error("the auth prefetch value has no in-memory representation")]
    Value(#[from] serde_json::Error),
}
pub type Missing = Vec<PrefetchEntry>;
// _missing_in_memory: only a stored value suppresses the fetch; None keeps the entry
#[must_use]
pub fn missing_in_memory(entries: &[PrefetchEntry], memory: &dyn PrefetchMemory) -> Missing {
    entries
        .iter()
        .filter(|entry| memory.get(&entry.key).is_none())
        .cloned()
        .collect()
}
// _set_in_memory: a None ttl reaches the memory tier without one, which applies its own default
pub fn set_in_memory(
    memory: &dyn PrefetchMemory,
    key: &str,
    value: &Json,
    ttl: Option<f64>,
) -> Result<(), PrefetchError> {
    let cached: CacheValue = serde_json::from_value(serde_json::to_value(value)?)?;
    memory.set(key, cached, ttl)?;
    Ok(())
}
// _read_redis_rows: one MGET of the sorted keys, duplicates kept; a failed pipeline reads as no rows
pub async fn read_redis_values(
    keys: &[String],
    redis: &dyn PrefetchRedis,
) -> Result<BTreeMap<String, Json>, PrefetchError> {
    let mut sorted = keys.to_vec();
    sorted.sort();
    let value = match redis.read(&sorted).await {
        Ok(value) => value,
        Err(RedisRowsError::Batch(source)) => {
            tracing::debug!(
                %source,
                "auth prefetch Redis read failed, filling from the database"
            );
            return Ok(BTreeMap::new());
        }
        Err(error) => return Err(PrefetchError::Redis(error)),
    };
    let Json::Object(entries) = value else {
        return Err(PrefetchError::Shape);
    };
    Ok(entries
        .into_iter()
        .filter(|(_key, entry)| !matches!(entry, &Json::Null))
        .collect())
}
// _fill_from_redis: Redis rows are cached as found, since the typed getters validate them
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
// every payload serializes before the first memory write, and the writes keep entry order
pub async fn write_back(
    writes: &[(PrefetchEntry, RowObject)],
    cache: &dyn PrefetchCache,
    redis: Option<&dyn PrefetchRedis>,
) -> Result<(), PrefetchError> {
    let payloads: Vec<(String, Json, Option<f64>)> = writes
        .iter()
        .map(|(entry, object)| Ok((entry.key.clone(), payload::encode(object)?, entry.ttl)))
        .collect::<Result<_, PayloadError>>()?;
    let memory = cache.ordinary_memory();
    let memory_default = cache.default_memory_ttl();
    for (key, payload, ttl) in &payloads {
        set_in_memory(memory, key, payload, ttl.or(memory_default))?;
    }
    let Some(connection) = redis else {
        return Ok(());
    };
    if connection.has_batch() {
        for (key, payload, ttl) in &payloads {
            connection.queue_set(key, payload, *ttl)?;
        }
        return Ok(());
    }
    connection
        .set_pipeline(&payloads)
        .await
        .map_err(PrefetchError::Redis)
}
// _fill_from_db: one SELECT for the missing kinds, one clock reading, one validation per kind
pub async fn fill_from_db(
    refs: &AuthObjectRefs,
    entries: &[PrefetchEntry],
    cache: &dyn PrefetchCache,
    database: &dyn PrefetchDatabase,
    clock: &dyn PrefetchClock,
) -> Result<(), PrefetchError> {
    if entries.is_empty() {
        return Ok(());
    }
    let mut kinds: Vec<RowKind> = entries.iter().map(|entry| entry.row).collect();
    kinds.sort();
    kinds.dedup();
    let rows = database.fetch_rows(refs, &kinds).await?;
    let refreshed_at = clock.now();
    let mut objects: BTreeMap<RowKind, RowObject> = BTreeMap::new();
    for kind in &kinds {
        if let Some(object) = validate_row(*kind, rows.get(kind), refreshed_at) {
            objects.insert(*kind, object);
        }
    }
    let writes: Vec<(PrefetchEntry, RowObject)> = entries
        .iter()
        .filter_map(|entry| {
            objects
                .get(&entry.row)
                .cloned()
                .map(|object| (entry.clone(), object))
        })
        .collect();
    if writes.is_empty() {
        return Ok(());
    }
    let redis = cache.redis();
    write_back(&writes, cache, redis.as_deref()).await
}
// time.time() at the point LiteLLM reads it: once, after the query
pub trait PrefetchClock: Send + Sync {
    fn now(&self) -> f64;
}
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemPrefetchClock;
impl PrefetchClock for SystemPrefetchClock {
    fn now(&self) -> f64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64()
    }
}
// litellm/constants.py: REGISTRY_ERROR_NEGATIVE_CACHE_TTL
pub const REGISTRY_ERROR_TTL: f64 = 30.0;
// a registry stored as a string is a sentinel, written with the shorter of the two registry TTLs
#[must_use]
pub const fn identity_memory_ttl(value: &Json, management_ttl: f64) -> f64 {
    if matches!(value, Json::Text(_)) {
        REGISTRY_ERROR_TTL.min(management_ttl)
    } else {
        management_ttl
    }
}
// all keys auth reads before it knows the key's owners; a resolved key skips the bearer hash
#[must_use]
pub fn identity_cache_keys(
    api_key: &str,
    end_user_id: Option<&str>,
    key_is_resolved: bool,
) -> Vec<String> {
    let mut keys = Vec::with_capacity(4);
    if !key_is_resolved {
        keys.push(hash_token(api_key));
    }
    if let Some(end_user) = end_user_id.filter(|id| !id.is_empty()) {
        keys.push(CacheKey::EndUserId(end_user).render());
        keys.push(CacheKey::EndUserRestrictedRegistry.render());
    }
    keys.push(CacheKey::ModelAccessGroupRegistry.render());
    keys
}
// the keys' owner rows are absent from memory and fetched in one MGET; no database read happens here
pub struct IdentityPrefetch<'cache> {
    pub cache: &'cache dyn PrefetchCache,
}
impl IdentityPrefetch<'_> {
    // the best-effort catch covers the Redis read and the TTL selection; later memory writes propagate
    pub async fn prefetch(&self, cache_keys: &[String]) -> Result<(), PrefetchError> {
        let Some(redis) = self.cache.redis() else {
            return Ok(());
        };
        let mut missing: Vec<String> = Vec::new();
        for key in cache_keys {
            if missing.contains(key) || self.cache.memory_for(key).get(key).is_some() {
                continue;
            }
            missing.push(key.clone());
        }
        if missing.is_empty() {
            return Ok(());
        }
        let found = match read_redis_values(&missing, redis.as_ref()).await {
            Ok(found) => found,
            Err(error) => {
                tracing::warn!(
                    %error,
                    "auth identity prefetch skipped, falling back to per-key lookups"
                );
                return Ok(());
            }
        };
        let management_ttl = self.cache.management_ttl();
        for key in &missing {
            if let Some(value) = found.get(key) {
                let ttl = identity_memory_ttl(value, management_ttl);
                let memory = self.cache.memory_for(key);
                set_in_memory(memory, key, value, Some(ttl))?;
            }
        }
        Ok(())
    }
}
