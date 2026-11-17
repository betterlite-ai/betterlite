use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, PoisonError};

use rust_decimal::prelude::ToPrimitive as _;
use serde::{Deserialize, Serialize};

use crate::cooldown_state::shared::SharedHealth;
use crate::routing_read_batch::local::{
    DEFAULT_LOCAL_TTL_SECONDS, DEFAULT_MAX_LAST_READS, DEFAULT_MAX_LOCAL_ENTRIES, LocalEntry,
    LocalTier, ReadTierSettings, TierState,
};
use crate::routing_value_store::{Clock, RouteKey, StoreError};

pub const CACHE_KEY: &str = "litellm:health_check:deployment_health_state";
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

fn default_is_healthy() -> bool {
    true
}

#[derive(Debug, thiserror::Error)]
pub enum HealthSnapshotError {
    #[error("the health snapshot key is not a usable routing key")]
    Key {
        #[source]
        source: StoreError,
    },
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
    pub fn new(
        staleness_threshold: f64,
        shared: Arc<dyn SharedHealth>,
        clock: Clock,
    ) -> Result<Self, HealthSnapshotError> {
        Ok(Self {
            key: RouteKey::parse(CACHE_KEY)
                .map_err(|source| HealthSnapshotError::Key { source })?,
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
        })
    }

    // set_deployment_health_states: merge this writer's states into the shared snapshot and prune the expired ones
    pub fn set_states(&self, states: &HealthSnapshot) -> Result<(), HealthSnapshotError> {
        let now = (self.clock)();
        let local = self.tier.batch_get(std::slice::from_ref(&self.key));
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
        let expires_at = ttl.mul_add(1.0_f64, (self.tier.clock)());
        let mut tier = self
            .tier
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        tier.entries.insert(
            self.key.clone(),
            LocalEntry {
                value: merged.clone(),
                expires_at,
            },
        );
        drop(tier);
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
        self.tier
            .apply(std::slice::from_ref(&self.key), &[None], &fetched);
        Some(snapshot)
    }

    // async_get_unhealthy_deployment_ids: memory first, then the shared snapshot
    #[must_use]
    pub async fn unhealthy(&self) -> BTreeSet<String> {
        let now = (self.clock)();
        let local = self.tier.batch_get(std::slice::from_ref(&self.key));
        let snapshot = match local.into_iter().next().flatten() {
            Some(snapshot) => Some(snapshot),
            None => self.backfilled(self.shared.read().await),
        };
        extract_unhealthy(snapshot.as_ref(), now, self.staleness_threshold)
    }

    // get_unhealthy_deployment_ids: the sync facade over the same extraction and backfill
    #[must_use]
    pub fn unhealthy_now(&self) -> BTreeSet<String> {
        let now = (self.clock)();
        let local = self.tier.batch_get(std::slice::from_ref(&self.key));
        let snapshot = match local.into_iter().next().flatten() {
            Some(snapshot) => Some(snapshot),
            None => self.backfilled(self.shared.read_raw()),
        };
        extract_unhealthy(snapshot.as_ref(), now, self.staleness_threshold)
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
