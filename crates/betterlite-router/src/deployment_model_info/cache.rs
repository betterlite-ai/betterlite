use super::types::DiscoveredDeploymentModelInfo;
use crate::model_catalog_lookup::cache::CacheClear;
use betterlite_openai::compatible_models::MODEL_INFO_REFRESH_SECONDS;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
pub const DISCOVERY_TTL_SECONDS: u64 = 2 * MODEL_INFO_REFRESH_SECONDS;
#[derive(Debug, Default)]
struct DiscoveryState {
    capacity: i64,
    entries: BTreeMap<String, (DiscoveredDeploymentModelInfo, f64)>,
}
pub struct DiscoveryCache {
    clock: Arc<dyn Fn() -> f64 + Send + Sync>,
    ttl_seconds: u64,
    state: Mutex<DiscoveryState>,
}
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
            }),
        }
    }
    #[must_use]
    pub fn for_deployments(deployments: usize) -> Self {
        let capacity = i64::try_from(deployments).unwrap_or(i64::MAX).max(1);
        let clock: Arc<dyn Fn() -> f64 + Send + Sync> = Arc::new(|| {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0.0, |elapsed| elapsed.as_secs_f64())
        });
        Self::new(capacity, DISCOVERY_TTL_SECONDS, clock)
    }
    #[must_use]
    pub fn get(&self, model_id: &str) -> Option<DiscoveredDeploymentModelInfo> {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let expires_at = state.entries.get(model_id)?.1;
        if (self.clock)() > expires_at {
            state.entries.remove(model_id);
            return None;
        }
        state
            .entries
            .get(model_id)
            .map(|(record, _expires_at)| record.clone())
    }
    pub fn store(&self, model_id: &str, record: DiscoveredDeploymentModelInfo) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if state.capacity <= 0 {
            return;
        }
        let now = (self.clock)();
        state
            .entries
            .retain(|_model_id, (_record, expires_at)| *expires_at > now);
        evict_to_capacity(&mut state);
        let expires_at = match state.entries.get(model_id) {
            Some((_record, expires_at)) if *expires_at > now => *expires_at,
            None | Some(_) => expiry_after(now, self.ttl_seconds),
        };
        let entry = (record, expires_at);
        state.entries.insert(model_id.to_owned(), entry);
    }
    pub fn delete(&self, model_id: &str) {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entries
            .remove(model_id);
    }
    pub fn resize(&self, deployments: usize) {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .capacity = i64::try_from(deployments).unwrap_or(i64::MAX).max(1);
    }
    #[must_use]
    pub fn report(&self) -> CacheReport {
        let state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        CacheReport {
            capacity: state.capacity,
            entries: state.entries.keys().cloned().collect(),
            expirations: state
                .entries
                .iter()
                .map(|(model_id, (_record, expires_at))| (model_id.clone(), *expires_at))
                .collect(),
        }
    }
}
impl CacheClear for DiscoveryCache {
    fn clear(&self) {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entries
            .clear();
    }
}
fn expiry_after(current: f64, ttl_seconds: u64) -> f64 {
    let started = Duration::try_from_secs_f64(current);
    let expires = started
        .ok()
        .and_then(|start| start.checked_add(Duration::from_secs(ttl_seconds)));
    expires.map_or(current, |moment| moment.as_secs_f64())
}
fn evict_to_capacity(state: &mut DiscoveryState) {
    let capacity = usize::try_from(state.capacity).unwrap_or(usize::MAX);
    while state.entries.len() >= capacity {
        let victim = state
            .entries
            .iter()
            .min_by(|left, right| left.1.1.total_cmp(&right.1.1))
            .map(|(model_id, _entry)| model_id.clone());
        let Some(model_id) = victim else {
            return;
        };
        state.entries.remove(&model_id);
    }
}
