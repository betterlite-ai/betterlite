use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};

use betterlite_context::dynamic_logging_cache::{Clock, lookup_marked, store_marked};
use betterlite_context::local_ttl_cache::{CacheError, LocalTtlCache};
use betterlite_db::managed_resource_rows::skills::SkillTable;

use super::StoreError;

// LiteLLM's InMemoryCache(max_size_in_memory=10000, default_ttl=60)
const CACHE_CAPACITY: i64 = 10_000;
const CACHE_TTL_SECONDS: f64 = 60.0;
const ITEM_KIB: i64 = 1024;

#[derive(Clone)]
enum CachedEntry {
    Missing,
    Found(Box<SkillTable>),
}

pub(crate) enum CachedRow {
    Miss,
    Missing,
    Row(Box<SkillTable>),
}

struct Inner {
    markers: LocalTtlCache,
    entries: BTreeMap<String, CachedEntry>,
}

pub(crate) struct SkillCache {
    inner: Mutex<Inner>,
}

impl SkillCache {
    pub(crate) fn new(clock: Clock) -> Result<Self, CacheError> {
        let markers = LocalTtlCache::with_clock(
            Some(CACHE_CAPACITY),
            Some(CACHE_TTL_SECONDS),
            Some(ITEM_KIB),
            ITEM_KIB,
            clock,
        )?;
        let inner = Mutex::new(Inner {
            markers,
            entries: BTreeMap::new(),
        });
        Ok(Self { inner })
    }

    // The cache holds the row object, not a serialized copy, the way LiteLLM's InMemoryCache does
    pub(crate) fn lookup(&self, skill_id: &str) -> Result<CachedRow, StoreError> {
        let mut guard = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        let inner = &mut *guard;
        let (cached, _) = lookup_marked(&mut inner.markers, &mut inner.entries, skill_id)?;
        drop(guard);
        Ok(match cached {
            None => CachedRow::Miss,
            Some(CachedEntry::Missing) => CachedRow::Missing,
            Some(CachedEntry::Found(row)) => CachedRow::Row(row),
        })
    }

    pub(crate) fn store(&self, skill_id: &str, row: Option<&SkillTable>) -> Result<(), StoreError> {
        let entry = row.map_or(CachedEntry::Missing, |found| {
            CachedEntry::Found(Box::new(found.clone()))
        });
        let mut guard = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        let inner = &mut *guard;
        store_marked(
            &mut inner.markers,
            &mut inner.entries,
            skill_id.to_owned(),
            entry,
            None,
        )?;
        drop(guard);
        Ok(())
    }
}
