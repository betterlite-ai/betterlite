use std::sync::{Arc, Mutex, PoisonError};

use betterlite_context::dynamic_logging_cache::Clock;
use betterlite_context::local_ttl_cache::{CacheError, CacheValue, LocalTtlCache};

// InMemoryCache returns None for a miss and for a cached None alike, so the untracked answer is stored as text
pub(crate) const NEGATIVE_OWNER_SENTINEL: &str = "__litellm_container_no_owner__";
pub(crate) const NEGATIVE_STORED_ID_SENTINEL: &str = "__litellm_container_no_stored_id__";

// InMemoryCache(max_size_in_memory=10000, default_ttl=60) and InMemoryCache(max_size_in_memory=2048, default_ttl=60)
const OWNER_CAPACITY: i64 = 10_000;
const STORED_ID_CAPACITY: i64 = 10_000;
const ALLOWED_CAPACITY: i64 = 2_048;
const CACHE_TTL_SECONDS: f64 = 60.0;
const ITEM_KIB: i64 = 1024;

// LiteLLM's _allowed_container_ids_cache_key: JSON encoding escapes the separators a joined key would collide on
pub(crate) fn allowed_ids_cache_key(owner_scopes: &[String]) -> Result<String, serde_json::Error> {
    let mut sorted: Vec<&str> = owner_scopes.iter().map(String::as_str).collect();
    sorted.sort_unstable();
    serde_json::to_string(&sorted)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum CachedText {
    Miss,
    Absent,
    Known(String),
}

// one InMemoryCache: the lock is taken here so the callers hold no guard
struct Slot(Mutex<LocalTtlCache>);

impl Slot {
    fn new(capacity: i64, clock: Clock) -> Result<Self, CacheError> {
        let cache = LocalTtlCache::with_clock(
            Some(capacity),
            Some(CACHE_TTL_SECONDS),
            Some(ITEM_KIB),
            ITEM_KIB,
            clock,
        )?;
        Ok(Self(Mutex::new(cache)))
    }

    fn with<Out>(&self, action: impl FnOnce(&mut LocalTtlCache) -> Out) -> Out {
        let mut guard = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let out = action(&mut guard);
        drop(guard);
        out
    }
}

// a text cache whose miss and whose "nothing there" answer differ: the second one is stored as a sentinel
pub(crate) struct SentinelCache {
    slot: Slot,
    sentinel: &'static str,
    blank_is_absent: bool,
}

impl SentinelCache {
    fn new(
        capacity: i64,
        clock: Clock,
        sentinel: &'static str,
        blank_is_absent: bool,
    ) -> Result<Self, CacheError> {
        Ok(Self {
            slot: Slot::new(capacity, clock)?,
            sentinel,
            blank_is_absent,
        })
    }

    pub(crate) fn read(&self, model_object_id: &str) -> Result<CachedText, CacheError> {
        let Some(text) = wrapped_text(self.slot.with(|cache| cache.get(model_object_id))?) else {
            return Ok(CachedText::Miss);
        };
        if text == self.sentinel {
            return Ok(CachedText::Absent);
        }
        Ok(CachedText::Known(text))
    }

    pub(crate) fn write(
        &self,
        model_object_id: &str,
        value: Option<&str>,
    ) -> Result<(), CacheError> {
        let stored = value
            .filter(|text| !(self.blank_is_absent && text.is_empty()))
            .unwrap_or(self.sentinel);
        let wrapped = CacheValue::Array(vec![CacheValue::Text(stored.to_owned())]);
        self.slot
            .with(|cache| cache.set(model_object_id, wrapped, None))
    }
}

pub(crate) struct ContainerCaches {
    pub(crate) owner: SentinelCache,
    pub(crate) stored_id: SentinelCache,
    allowed: Slot,
}

impl ContainerCaches {
    pub(crate) fn new(clock: Clock) -> Result<Self, CacheError> {
        Ok(Self {
            owner: SentinelCache::new(
                OWNER_CAPACITY,
                Arc::clone(&clock),
                NEGATIVE_OWNER_SENTINEL,
                false,
            )?,
            stored_id: SentinelCache::new(
                STORED_ID_CAPACITY,
                Arc::clone(&clock),
                NEGATIVE_STORED_ID_SENTINEL,
                true,
            )?,
            allowed: Slot::new(ALLOWED_CAPACITY, clock)?,
        })
    }

    pub(crate) fn allowed(&self, cache_key: &str) -> Result<Option<Vec<String>>, CacheError> {
        let Some(CacheValue::Array(items)) = self.allowed.with(|cache| cache.get(cache_key))?
        else {
            return Ok(None);
        };
        Ok(Some(items.into_iter().filter_map(cached_text).collect()))
    }

    pub(crate) fn store_allowed(
        &self,
        cache_key: &str,
        allowed: &[String],
    ) -> Result<(), CacheError> {
        let items = allowed
            .iter()
            .map(|model_object_id| CacheValue::Text(model_object_id.clone()))
            .collect();
        self.allowed
            .with(|cache| cache.set(cache_key, CacheValue::Array(items), None))
    }

    pub(crate) fn forget_allowed(&self, cache_key: &str) {
        self.allowed.with(|cache| cache.delete(cache_key));
    }
}

// LocalTtlCache::get decodes a stored text as JSON, so a text is kept inside a one-item array and comes back unchanged
fn wrapped_text(cached: Option<CacheValue>) -> Option<String> {
    let Some(CacheValue::Array(items)) = cached else {
        return None;
    };
    items.into_iter().next().and_then(cached_text)
}

fn cached_text(value: CacheValue) -> Option<String> {
    match value {
        CacheValue::Text(text) => Some(text),
        CacheValue::Credential(_)
        | CacheValue::Bytes(_)
        | CacheValue::Null
        | CacheValue::Boolean(_)
        | CacheValue::Integer(_)
        | CacheValue::Float(_)
        | CacheValue::Array(_)
        | CacheValue::Object(_) => None,
    }
}
