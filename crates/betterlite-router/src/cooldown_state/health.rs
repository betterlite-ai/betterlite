use std::collections::{BTreeMap, BTreeSet};
use std::slice::from_ref;
use std::sync::{Arc, Mutex, PoisonError};

use rust_decimal::prelude::ToPrimitive as _;
use serde::{Deserialize, Serialize};

use crate::cooldown_state::shared::SharedHealth;
use crate::routing_read_batch::local::{
    DEFAULT_LOCAL_TTL_SECONDS, DEFAULT_MAX_LAST_READS, DEFAULT_MAX_LOCAL_ENTRIES, LocalEntry,
    LocalTier, ReadTierSettings, TierState,
};
use crate::routing_value_store::{Clock, MAX_KEY_BYTES, RouteKey};

pub const CACHE_KEY: &str = "litellm:health_check:deployment_health_state";
// the health state key is built without parse's error, so its size is proven here instead
const _: () = assert!(CACHE_KEY.len() <= MAX_KEY_BYTES);
// LiteLLM keeps a state until staleness_threshold * 1.5 seconds have passed, which is also the entry's TTL
pub const STALENESS_MULTIPLIER: f64 = 1.5;

pub type HealthSnapshot = BTreeMap<String, HealthState>;

// DeploymentHealthStateValue: a probe's verdict on one deployment, beside the moment it was taken, with unknown fields dropped rather than refused
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HealthState {
    #[serde(default = "default_is_healthy")]
    pub is_healthy: bool,
    #[serde(default)]
    pub timestamp: f64,
    #[serde(default)]
    pub reason: String,
}

const fn default_is_healthy() -> bool {
    true
}

#[derive(Debug, thiserror::Error)]
pub enum HealthSnapshotError {
    #[error("the health state staleness of {seconds} seconds has no whole-second expiry")]
    Expiry { seconds: f64 },
}

// DeploymentHealthCache: one shared snapshot of every deployment's health, mirrored in a local tier
pub struct DeploymentHealthCache {
    key: RouteKey,
    tier: LocalTier<HealthSnapshot>,
    shared: Arc<dyn SharedHealth>,
    staleness_threshold: f64,
    clock: Clock,
}

impl DeploymentHealthCache {
    #[must_use]
    pub fn new(staleness_threshold: f64, shared: Arc<dyn SharedHealth>, clock: Clock) -> Self {
        Self {
            key: RouteKey(CACHE_KEY.to_owned()),
            tier: LocalTier {
                settings: ReadTierSettings {
                    backfill_ttl_seconds: DEFAULT_LOCAL_TTL_SECONDS,
                    read_expiry_seconds: 0.0,
                    max_entries: DEFAULT_MAX_LOCAL_ENTRIES,
                    max_last_reads: DEFAULT_MAX_LAST_READS,
                },
                clock: Arc::clone(&clock),
                state: Mutex::new(TierState::default()),
            },
            shared,
            staleness_threshold,
            clock,
        }
    }

    // set_deployment_health_states: merge this writer's states into the shared snapshot and prune the expired ones
    pub fn set_states(&self, states: &HealthSnapshot) -> Result<(), HealthSnapshotError> {
        let now = (self.clock)();
        let local = self.tier.batch_get(from_ref(&self.key));
        let existing = self
            .shared
            .read_raw()
            .or_else(|| local.into_iter().next().flatten());
        let mut merged = existing.unwrap_or_default();
        for (model_id, state) in states {
            merged.insert(model_id.clone(), state.clone());
        }
        let expiry = self.staleness_threshold.mul_add(STALENESS_MULTIPLIER, 0.0);
        merged.retain(|_model_id, state| state.timestamp.mul_add(-1.0_f64, now) < expiry);
        let ttl = expiry.trunc();
        let seconds = ttl
            .to_i64()
            .ok_or(HealthSnapshotError::Expiry { seconds: expiry })?;
        let tier = &self.tier;
        let mut state = tier.state.lock().unwrap_or_else(PoisonError::into_inner);
        // InMemoryCache.set_cache prunes a TTL that ends exactly now before it writes, so such an expiry is replaced
        let kept_expiry = state
            .entries
            .get(&self.key)
            .map(|entry| entry.expires_at)
            .filter(|expires_at| *expires_at > now);
        let expires_at = kept_expiry.unwrap_or_else(|| ttl.mul_add(1.0_f64, now));
        state.entries.insert(
            self.key.clone(),
            LocalEntry {
                value: merged.clone(),
                expires_at,
            },
        );
        drop(state);
        self.shared.write(&merged, seconds);
        Ok(())
    }

    // a snapshot the shared read answered lands in the memory tier, so the next read answers without one
    fn backfilled(&self, snapshot: Option<HealthSnapshot>) -> Option<HealthSnapshot> {
        let Some(snapshot) = snapshot else {
            return None;
        };
        let mut fetched = BTreeMap::new();
        fetched.insert(self.key.clone(), snapshot.clone());
        self.tier.apply(from_ref(&self.key), &[None], &fetched);
        Some(snapshot)
    }

    // async_get_unhealthy_deployment_ids: memory first, and the shared snapshot only when memory has none
    #[must_use]
    pub async fn unhealthy(&self) -> BTreeSet<String> {
        let local = self.tier.batch_get(from_ref(&self.key));
        let fetched = if local.first().is_some_and(Option::is_some) {
            None
        } else {
            self.shared.read().await
        };
        self.unhealthy_from(&local, fetched)
    }

    // get_unhealthy_deployment_ids: the sync facade, over the memory tier or a raw shared read
    #[must_use]
    pub fn unhealthy_now(&self) -> BTreeSet<String> {
        let snapshot = self.stored_or_raw();
        let now = (self.clock)();
        extract_unhealthy(snapshot.as_ref(), now, self.staleness_threshold)
    }

    // the memory tier's snapshot, else the raw shared one, which lands in memory when it answers
    fn stored_or_raw(&self) -> Option<HealthSnapshot> {
        let local = self.tier.batch_get(from_ref(&self.key));
        let stored = local.into_iter().next().flatten();
        stored.or_else(|| self.backfilled(self.shared.read_raw()))
    }

    // the local tier's absolute expiry for the one snapshot key, the way InMemoryCache.ttl_dict answers it
    #[must_use]
    pub fn local_expiry(&self) -> Option<f64> {
        let now = (self.clock)();
        let tier = &self.tier;
        let mut state = tier.state.lock().unwrap_or_else(PoisonError::into_inner);
        let expiry = state
            .entries
            .get(&self.key)
            .map(|entry| entry.expires_at)
            .filter(|expiry| now <= *expiry);
        if expiry.is_none() {
            state.entries.remove(&self.key);
        }
        expiry
    }

    // _extract_unhealthy_ids: the stored snapshot wins, and a fetched one lands in memory when it answers
    fn unhealthy_from(
        &self,
        local: &[Option<HealthSnapshot>],
        fetched: Option<HealthSnapshot>,
    ) -> BTreeSet<String> {
        let stored = local.first().cloned().flatten();
        let snapshot = stored.or_else(|| self.backfilled(fetched));
        extract_unhealthy(snapshot.as_ref(), (self.clock)(), self.staleness_threshold)
    }
}

// _extract_unhealthy_ids: the deployments an explicit unhealthy verdict still covers, never one whose verdict went stale
#[must_use]
pub fn extract_unhealthy(
    snapshot: Option<&HealthSnapshot>,
    now: f64,
    staleness_threshold: f64,
) -> BTreeSet<String> {
    let Some(snapshot) = snapshot else {
        return BTreeSet::new();
    };
    snapshot
        .iter()
        .filter(|(_model_id, state)| {
            !state.is_healthy && state.timestamp.mul_add(-1.0_f64, now) < staleness_threshold
        })
        .map(|(model_id, _state)| model_id.clone())
        .collect()
}
