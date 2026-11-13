pub mod local;
pub mod store;

mod prefetch;
mod read;
mod tests;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use crate::routing_value_store::RouteKey;

use self::local::{LocalTier, Reservation};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct UsageReadKeys {
    pub tpm: Vec<RouteKey>,
    pub rpm: Vec<RouteKey>,
}

impl UsageReadKeys {
    #[must_use]
    pub fn combined(&self) -> Vec<RouteKey> {
        let mut keys = Vec::with_capacity(self.tpm.len().saturating_add(self.rpm.len()));
        keys.extend(self.tpm.iter().cloned());
        keys.extend(self.rpm.iter().cloned());
        keys
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RoutingReadKeys {
    pub cooldown: Vec<RouteKey>,
    pub usage: Option<UsageReadKeys>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PrefetchedUsage<V> {
    pub keys: Vec<RouteKey>,
    pub values: Option<BTreeMap<RouteKey, Option<V>>>,
}

impl<V: Clone> PrefetchedUsage<V> {
    #[must_use]
    pub fn covers(&self, keys: &[RouteKey]) -> bool {
        for key in keys {
            if !self.keys.contains(key) {
                return false;
            }
        }
        true
    }

    #[must_use]
    pub fn values_for(&self, keys: &[RouteKey]) -> Option<Vec<Option<V>>> {
        let values = self.values.as_ref()?;
        Some(
            keys.iter()
                .map(|key| values.get(key).cloned().flatten())
                .collect(),
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RoutingReadResults<V> {
    pub cooldown: Vec<Option<V>>,
    pub usage: Option<Vec<Option<V>>>,
    pub warning: Option<store::ReadWarning>,
}

pub struct ReadTiers<V> {
    pub cooldown: Arc<LocalTier<V>>,
    pub usage: Option<Arc<LocalTier<V>>>,
}

pub struct RoutingReadSession<'read, S, V> {
    pub store: &'read S,
    pub tiers: &'read ReadTiers<V>,
    pub request: &'read mut RequestReads<V>,
}

pub struct RequestReads<V> {
    pub prefetch: Option<RoutingPrefetch<V>>,
}

impl<V> Default for RequestReads<V> {
    fn default() -> Self {
        Self { prefetch: None }
    }
}

impl<V> RequestReads<V> {
    pub fn take(&mut self, needed: &[RouteKey]) -> Option<RoutingPrefetch<V>> {
        let prefetch = self.prefetch.take()?;
        if needed.iter().all(|key| prefetch.keys.contains(key)) {
            return Some(prefetch);
        }
        None
    }
}

pub struct RoutingPrefetch<V> {
    pub(crate) keys: BTreeSet<RouteKey>,
    pub(crate) fetched: Vec<RouteKey>,
    pub(crate) reservations: Vec<TierReservation<V>>,
}

pub(crate) struct TierReservation<V> {
    pub(crate) tier: Arc<LocalTier<V>>,
    pub(crate) reservation: Reservation,
}

impl<V> Drop for RoutingPrefetch<V> {
    fn drop(&mut self) {
        self.release();
    }
}

impl<V> RoutingPrefetch<V> {
    pub fn release(&mut self) {
        for entry in &self.reservations {
            entry.tier.release(&entry.reservation);
        }
        self.reservations.clear();
    }
}

pub struct RoutingReadBatch<V> {
    pub usage: Option<UsageReadKeys>,
    pub prefetched_usage: Option<PrefetchedUsage<V>>,
}

impl<V> Default for RoutingReadBatch<V> {
    fn default() -> Self {
        Self {
            usage: None,
            prefetched_usage: None,
        }
    }
}

impl<V> RoutingReadBatch<V> {
    #[must_use]
    pub fn for_strategy(usage: Option<UsageReadKeys>, prefetch_armed: bool) -> Option<Self> {
        match usage {
            Some(usage_keys) => Some(Self {
                usage: Some(usage_keys),
                prefetched_usage: None,
            }),
            None if prefetch_armed => Some(Self {
                usage: None,
                prefetched_usage: None,
            }),
            None => None,
        }
    }
}
