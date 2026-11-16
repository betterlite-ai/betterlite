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
pub(crate) enum CachedOwner {
    Miss,
    Untracked,
    Owned(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum CachedStoredId {
    Miss,
    Absent,
    Known(String),
}

pub(crate) struct ContainerCaches {
    owner: Mutex<LocalTtlCache>,
    stored_id: Mutex<LocalTtlCache>,
    allowed: Mutex<LocalTtlCache>,
}

impl ContainerCaches {
    pub(crate) fn new(clock: Clock) -> Result<Self, CacheError> {
        Ok(Self {
            owner: Mutex::new(cache(OWNER_CAPACITY, Arc::clone(&clock))?),
            stored_id: Mutex::new(cache(STORED_ID_CAPACITY, Arc::clone(&clock))?),
            allowed: Mutex::new(cache(ALLOWED_CAPACITY, clock)?),
        })
    }

    pub(crate) fn owner(&self, model_object_id: &str) -> Result<CachedOwner, CacheError> {
        let mut guard = self.owner.lock().unwrap_or_else(PoisonError::into_inner);
        let cached = guard.get(model_object_id)?;
        drop(guard);
        let Some(text) = wrapped_text(cached) else {
            return Ok(CachedOwner::Miss);
        };
        Ok(match text.as_str() {
            NEGATIVE_OWNER_SENTINEL => CachedOwner::Untracked,
            _ => CachedOwner::Owned(text),
        })
    }

    pub(crate) fn store_owner(
        &self,
        model_object_id: &str,
        owner: Option<&str>,
    ) -> Result<(), CacheError> {
        let stored = owner.map_or_else(|| NEGATIVE_OWNER_SENTINEL.to_owned(), str::to_owned);
        let mut guard = self.owner.lock().unwrap_or_else(PoisonError::into_inner);
        let written = guard.set(
            model_object_id,
            CacheValue::Array(vec![CacheValue::Text(stored)]),
            None,
        );
        drop(guard);
        written
    }

    pub(crate) fn stored_id(&self, model_object_id: &str) -> Result<CachedStoredId, CacheError> {
        let mut guard = self
            .stored_id
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let cached = guard.get(model_object_id)?;
        drop(guard);
        let Some(text) = wrapped_text(cached) else {
            return Ok(CachedStoredId::Miss);
        };
        Ok(match text.as_str() {
            NEGATIVE_STORED_ID_SENTINEL => CachedStoredId::Absent,
            _ => CachedStoredId::Known(text),
        })
    }

    pub(crate) fn store_stored_id(
        &self,
        model_object_id: &str,
        stored_id: Option<&str>,
    ) -> Result<(), CacheError> {
        let stored = stored_id
            .filter(|found| !found.is_empty())
            .map_or_else(|| NEGATIVE_STORED_ID_SENTINEL.to_owned(), str::to_owned);
        let mut guard = self
            .stored_id
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let written = guard.set(
            model_object_id,
            CacheValue::Array(vec![CacheValue::Text(stored)]),
            None,
        );
        drop(guard);
        written
    }

    pub(crate) fn allowed(&self, cache_key: &str) -> Result<Option<Vec<String>>, CacheError> {
        let mut guard = self.allowed.lock().unwrap_or_else(PoisonError::into_inner);
        let cached = guard.get(cache_key)?;
        drop(guard);
        let Some(CacheValue::Array(items)) = cached else {
            return Ok(None);
        };
        let allowed = items.into_iter().filter_map(cached_text).collect();
        Ok(Some(allowed))
    }

    pub(crate) fn store_allowed(
        &self,
        cache_key: &str,
        allowed: &[String],
    ) -> Result<(), CacheError> {
        let items: Vec<CacheValue> = allowed
            .iter()
            .map(|model_object_id| CacheValue::Text(model_object_id.clone()))
            .collect();
        let mut guard = self.allowed.lock().unwrap_or_else(PoisonError::into_inner);
        let written = guard.set(cache_key, CacheValue::Array(items), None);
        drop(guard);
        written
    }

    pub(crate) fn forget_allowed(&self, cache_key: &str) {
        let mut guard = self.allowed.lock().unwrap_or_else(PoisonError::into_inner);
        guard.delete(cache_key);
        drop(guard);
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

fn cache(capacity: i64, clock: Clock) -> Result<LocalTtlCache, CacheError> {
    LocalTtlCache::with_clock(
        Some(capacity),
        Some(CACHE_TTL_SECONDS),
        Some(ITEM_KIB),
        ITEM_KIB,
        clock,
    )
}
