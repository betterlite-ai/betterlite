use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use crate::routing_value_store::RouteKey;

pub const DEFAULT_LOCAL_TTL_SECONDS: f64 = 600.0;
pub const DEFAULT_READ_EXPIRY_SECONDS: f64 = 10.0;
pub const DEFAULT_MAX_LOCAL_ENTRIES: usize = 200;
pub const DEFAULT_MAX_LAST_READS: usize = 1_000;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ReadTierSettings {
    pub backfill_ttl_seconds: f64,
    pub read_expiry_seconds: f64,
    pub max_entries: usize,
    pub max_last_reads: usize,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Reservation {
    pub(crate) due: Vec<RouteKey>,
    pub(crate) previous: BTreeMap<RouteKey, Option<f64>>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PreparedRead<V> {
    pub local: Vec<Option<V>>,
    pub reservation: Reservation,
}

pub struct LocalEntry<V> {
    pub value: V,
    pub expires_at: f64,
}

pub struct TierState<V> {
    pub entries: BTreeMap<RouteKey, LocalEntry<V>>,
    pub last_reads: BTreeMap<RouteKey, f64>,
}

impl<V> Default for TierState<V> {
    fn default() -> Self {
        Self {
            entries: BTreeMap::new(),
            last_reads: BTreeMap::new(),
        }
    }
}

pub struct LocalTier<V> {
    pub settings: ReadTierSettings,
    pub clock: Arc<dyn Fn() -> f64 + Send + Sync>,
    pub state: Mutex<TierState<V>>,
}

impl<V> LocalTier<V> {
    pub fn release(&self, reservation: &Reservation) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        for (key, previous) in &reservation.previous {
            match previous {
                Some(time) => {
                    state.last_reads.insert(key.clone(), *time);
                }
                None => {
                    state.last_reads.remove(key);
                }
            }
        }
    }
}

impl<V> LocalTier<V>
where
    V: Clone,
{
    #[must_use]
    pub fn batch_get(&self, keys: &[RouteKey]) -> Vec<Option<V>> {
        let now = (self.clock)();
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        keys.iter().map(|key| read(&mut state, key, now)).collect()
    }

    #[must_use]
    pub fn prepare(&self, keys: &[RouteKey]) -> PreparedRead<V> {
        let now = (self.clock)();
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let mut local = Vec::with_capacity(keys.len());
        let mut due = Vec::new();
        let mut previous = BTreeMap::new();
        for key in keys {
            let value = read(&mut state, key, now);
            local.push(value.clone());
            if value.is_some() {
                continue;
            }
            let last = state.last_reads.get(key).copied();
            let due_now = last.is_none_or(|earlier| {
                earlier.mul_add(-1.0_f64, now) >= self.settings.read_expiry_seconds
            });
            if due_now {
                previous.insert(key.clone(), last);
                state.last_reads.insert(key.clone(), now);
                due.push(key.clone());
            }
        }
        enforce_read_capacity(&mut state, self.settings.max_last_reads);
        drop(state);
        PreparedRead {
            local,
            reservation: Reservation { due, previous },
        }
    }

    pub fn apply(
        &self,
        keys: &[RouteKey],
        local: &[Option<V>],
        fetched: &BTreeMap<RouteKey, V>,
    ) -> Vec<Option<V>> {
        let now = (self.clock)();
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let mut merged = Vec::with_capacity(keys.len());
        for (key, stored) in keys.iter().zip(local) {
            match fetched.get(key) {
                Some(value) => {
                    insert(
                        &mut state,
                        key,
                        value,
                        now,
                        self.settings.backfill_ttl_seconds,
                    );
                    merged.push(Some(value.clone()));
                }
                None => merged.push(stored.clone()),
            }
        }
        enforce_entry_capacity(&mut state, now, self.settings.max_entries);
        merged
    }

    pub fn backfill(&self, keys: &[RouteKey], values: &BTreeMap<RouteKey, V>) {
        let now = (self.clock)();
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        for key in keys {
            let Some(value) = values.get(key) else {
                continue;
            };
            if read(&mut state, key, now).is_none() {
                insert(
                    &mut state,
                    key,
                    value,
                    now,
                    self.settings.backfill_ttl_seconds,
                );
            }
        }
        enforce_entry_capacity(&mut state, now, self.settings.max_entries);
    }
}

fn read<V>(state: &mut TierState<V>, key: &RouteKey, now: f64) -> Option<V>
where
    V: Clone,
{
    let expired = state
        .entries
        .get(key)
        .is_some_and(|entry| now > entry.expires_at);
    if expired {
        state.entries.remove(key);
        return None;
    }
    state.entries.get(key).map(|entry| entry.value.clone())
}

fn insert<V>(state: &mut TierState<V>, key: &RouteKey, value: &V, now: f64, ttl: f64)
where
    V: Clone,
{
    state.entries.insert(
        key.clone(),
        LocalEntry {
            value: value.clone(),
            expires_at: now.mul_add(1.0_f64, ttl),
        },
    );
}

pub(super) fn push_unique(keys: &mut Vec<RouteKey>, key: &RouteKey) {
    if !keys.contains(key) {
        keys.push(key.clone());
    }
}

fn enforce_entry_capacity<V>(state: &mut TierState<V>, now: f64, max_entries: usize) {
    if state.entries.len() <= max_entries {
        return;
    }
    state.entries.retain(|_, entry| now <= entry.expires_at);
    while state.entries.len() > max_entries {
        let Some(oldest) = state
            .entries
            .iter()
            .min_by(|left, right| left.1.expires_at.total_cmp(&right.1.expires_at))
            .map(|(key, _)| key.clone())
        else {
            return;
        };
        state.entries.remove(&oldest);
    }
}

fn enforce_read_capacity<V>(state: &mut TierState<V>, max_last_reads: usize) {
    while state.last_reads.len() > max_last_reads {
        let Some(oldest) = state
            .last_reads
            .iter()
            .min_by(|left, right| left.1.total_cmp(right.1))
            .map(|(key, _)| key.clone())
        else {
            return;
        };
        state.last_reads.remove(&oldest);
    }
}
