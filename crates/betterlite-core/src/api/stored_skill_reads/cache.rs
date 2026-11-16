use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};

use betterlite_context::dynamic_logging_cache::{Clock, lookup_marked, store_marked};
use betterlite_context::local_ttl_cache::{CacheError, LocalTtlCache};
use betterlite_db::managed_resource_rows::skills::SkillTable;

use super::error::StoredSkillReadError;

// LiteLLM's InMemoryCache(max_size_in_memory=10000, default_ttl=60)
const DEFAULT_MAX_SIZE_IN_MEMORY: i64 = 10_000;
const DEFAULT_TTL_SECONDS: f64 = 60.0;
const ITEM_KIB: i64 = 1024;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SkillCacheLimits {
    pub max_size_in_memory: i64,
    pub default_ttl: f64,
}

impl Default for SkillCacheLimits {
    fn default() -> Self {
        Self {
            max_size_in_memory: DEFAULT_MAX_SIZE_IN_MEMORY,
            default_ttl: DEFAULT_TTL_SECONDS,
        }
    }
}

pub(crate) enum CachedSkill {
    Miss,
    Missing,
    Row(Box<SkillTable>),
}

struct Rows {
    markers: LocalTtlCache,
    values: BTreeMap<String, Option<SkillTable>>,
}

pub(crate) struct SkillRowCache {
    rows: Mutex<Rows>,
}

impl SkillRowCache {
    pub(crate) fn new(limits: SkillCacheLimits, clock: Clock) -> Result<Self, CacheError> {
        let markers = LocalTtlCache::with_clock(
            Some(limits.max_size_in_memory),
            Some(limits.default_ttl),
            Some(ITEM_KIB),
            ITEM_KIB,
            clock,
        )?;
        let rows = Rows {
            markers,
            values: BTreeMap::new(),
        };
        Ok(Self {
            rows: Mutex::new(rows),
        })
    }

    pub(crate) fn lookup(&self, skill_id: &str) -> Result<CachedSkill, StoredSkillReadError> {
        let found = {
            let mut rows = self.rows.lock().unwrap_or_else(PoisonError::into_inner);
            lookup_marked(&mut rows.markers, &mut rows.values, skill_id)?
        };
        Ok(match found.0 {
            None => CachedSkill::Miss,
            Some(None) => CachedSkill::Missing,
            Some(Some(row)) => CachedSkill::Row(Box::new(row)),
        })
    }

    pub(crate) fn store(
        &self,
        skill_id: &str,
        skill: Option<&SkillTable>,
    ) -> Result<(), StoredSkillReadError> {
        let mut rows = self.rows.lock().unwrap_or_else(PoisonError::into_inner);
        let value = skill.cloned();
        let (_evicted, _replaced) = store_marked(
            &mut rows.markers,
            &mut rows.values,
            skill_id.to_owned(),
            value,
            None,
        )?;
        drop(rows);
        Ok(())
    }
}
