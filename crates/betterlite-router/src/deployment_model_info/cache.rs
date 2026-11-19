use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use betterlite_openai::compatible_models::MODEL_INFO_REFRESH_SECONDS;

use crate::model_catalog_lookup::cache::CacheClear;

use super::types::DiscoveredDeploymentModelInfo;

// Router.__init__ sizes the cache to max(len(model_list), 1) and gives it default_ttl=2 * MODEL_INFO_REFRESH_SECONDS.
pub const DISCOVERY_TTL_SECONDS: u64 = 2 * MODEL_INFO_REFRESH_SECONDS;

#[derive(Debug, Default)]
struct DiscoveryState {
    capacity: i64,
    entries: BTreeMap<String, DiscoveredDeploymentModelInfo>,
    expirations: BTreeMap<String, f64>,
}

// The cache a refresh fills: LiteLLM's InMemoryCache semantics for capacity, ttl and eviction.
pub struct DiscoveryCache {
    clock: Arc<dyn Fn() -> f64 + Send + Sync>,
    ttl_seconds: u64,
    state: Mutex<DiscoveryState>,
}

// What the cache holds, for a caller that reports a refresh's outcome and for the tests that pin its eviction.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CacheReport {
    pub capacity: i64,
    pub entries: Vec<String>,
    pub expirations: Vec<(String, f64)>,
}

impl DiscoveryCache {
    #[must_use]
    pub const fn new(
        capacity: i64,
        ttl_seconds: u64,
        clock: Arc<dyn Fn() -> f64 + Send + Sync>,
    ) -> Self {
        Self {
            clock,
            ttl_seconds,
            state: Mutex::new(DiscoveryState {
                capacity,
                entries: BTreeMap::new(),
                expirations: BTreeMap::new(),
            }),
        }
    }

    // The cache Router.__init__ builds for a model list of this size.
    #[must_use]
    pub fn for_deployments(deployments: usize) -> Self {
        Self::new(
            i64::try_from(deployments).unwrap_or(i64::MAX).max(1),
            DISCOVERY_TTL_SECONDS,
            Arc::new(system_seconds),
        )
    }

    fn locked(&self) -> MutexGuard<'_, DiscoveryState> {
        match self.state.lock() {
            Ok(state) => state,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    // InMemoryCache.get_cache: an entry past its ttl is dropped, one exactly at it is still answered.
    #[must_use]
    pub fn get(&self, model_id: &str) -> Option<DiscoveredDeploymentModelInfo> {
        let mut state = self.locked();
        if !state.entries.contains_key(model_id) {
            return None;
        }
        let now = (self.clock)();
        let expired = state
            .expirations
            .get(model_id)
            .is_some_and(|expires_at| now > *expires_at);
        if expired {
            state.entries.remove(model_id);
            state.expirations.remove(model_id);
            return None;
        }
        state.entries.get(model_id).cloned()
    }

    // InMemoryCache.set_cache: evict, keep an unexpired ttl for this id, otherwise push a fresh one.
    pub fn store(&self, model_id: &str, record: DiscoveredDeploymentModelInfo) {
        let mut state = self.locked();
        if state.capacity <= 0 {
            return;
        }
        let now = (self.clock)();
        evict_expired(&mut state, now);
        evict_to_capacity(&mut state);
        let current = (self.clock)();
        let keeps_ttl = state
            .expirations
            .get(model_id)
            .is_some_and(|expires_at| *expires_at > current);
        state.entries.insert(model_id.to_owned(), record);
        if !keeps_ttl {
            let expires_at = expiry_after(current, self.ttl_seconds);
            state.expirations.insert(model_id.to_owned(), expires_at);
        }
    }

    pub fn delete(&self, model_id: &str) {
        let mut state = self.locked();
        state.entries.remove(model_id);
        state.expirations.remove(model_id);
    }

    // Router._arefresh_deployment_model_info sizes the cache to the current model list before each write.
    pub fn resize(&self, deployments: usize) {
        let capacity = i64::try_from(deployments).unwrap_or(i64::MAX).max(1);
        let mut state = self.locked();
        state.capacity = capacity;
    }

    #[must_use]
    pub fn report(&self) -> CacheReport {
        let state = self.locked();
        CacheReport {
            capacity: state.capacity,
            entries: state.entries.keys().cloned().collect(),
            expirations: state
                .expirations
                .iter()
                .map(|(model_id, expires_at)| (model_id.clone(), *expires_at))
                .collect(),
        }
    }
}

impl CacheClear for DiscoveryCache {
    // InMemoryCache.flush_cache drops every entry and expiration and keeps the configured capacity.
    fn clear(&self) {
        let mut state = self.locked();
        state.entries.clear();
        state.expirations.clear();
    }
}

// clock() + ttl without float arithmetic: a Duration add keeps the value exact for whole-second clocks.
fn expiry_after(current: f64, ttl_seconds: u64) -> f64 {
    let start = Duration::try_from_secs_f64(current).ok();
    let expiry = start
        .and_then(|start| start.checked_add(Duration::from_secs(ttl_seconds)))
        .map(Duration::as_secs_f64);
    expiry.unwrap_or(current)
}

fn system_seconds() -> f64 {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(elapsed) => elapsed.as_secs_f64(),
        Err(_before_epoch) => 0.0,
    }
}

fn evict_expired(state: &mut DiscoveryState, now: f64) {
    let expired: Vec<String> = state
        .expirations
        .iter()
        .filter(|(_, expires_at)| **expires_at <= now)
        .map(|(model_id, _)| model_id.clone())
        .collect();
    for model_id in expired {
        state.entries.remove(&model_id);
        state.expirations.remove(&model_id);
    }
}

// InMemoryCache.evict_cache: while the dict is at capacity, drop the earliest (expiration, key) pair.
fn evict_to_capacity(state: &mut DiscoveryState) {
    let capacity = usize::try_from(state.capacity).unwrap_or(usize::MAX);
    while state.entries.len() >= capacity {
        let Some(victim) = earliest(state) else {
            return;
        };
        state.entries.remove(&victim);
        state.expirations.remove(&victim);
    }
}

fn earliest(state: &DiscoveryState) -> Option<String> {
    let mut chosen: Option<(&String, f64)> = None;
    for (model_id, expires_at) in &state.expirations {
        let replace = match chosen {
            None => true,
            Some((name, expiry)) => match expires_at.partial_cmp(&expiry) {
                Some(Ordering::Less) => true,
                Some(Ordering::Equal) => model_id < name,
                Some(Ordering::Greater) | None => false,
            },
        };
        if replace {
            chosen = Some((model_id, *expires_at));
        }
    }
    chosen.map(|(model_id, _expires_at)| model_id.clone())
}
