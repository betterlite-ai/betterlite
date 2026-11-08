use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use betterlite_context::local_ttl_cache::{CacheError, CacheValue, LocalTtlCache};
use serde_json::Number;

use super::{BackendError, Clock, MemorySettings, MemoryState};

pub(super) fn oldest_keys(expiries: &BTreeMap<String, f64>, limit: usize) -> Vec<String> {
    let mut entries = expiries.iter().collect::<Vec<(&String, &f64)>>();
    entries.sort_by(|left, right| left.1.total_cmp(right.1).then_with(|| left.0.cmp(right.0)));
    entries
        .into_iter()
        .take(limit)
        .map(|(key, _)| key.clone())
        .collect()
}

pub(super) fn read(state: &mut MemoryState, key: &str) -> Result<Option<CacheValue>, BackendError> {
    let value = state.values.get(key)?;
    if value.is_none() {
        state.expiries.remove(key);
    }
    Ok(value)
}

pub(super) fn write(
    state: &mut MemoryState,
    settings: &MemorySettings,
    key: &str,
    value: CacheValue,
    ttl: Option<f64>,
) -> Result<(), BackendError> {
    let now = read_clock(&settings.clock)?;
    state.expiries.retain(|_, expiry| *expiry > now);
    evict_to_capacity(&mut state.expiries, settings.limits.capacity);
    let previous = state.expiries.get(key).copied();
    let accepted = settings.limits.capacity != 0 && accepts(settings, key, &value)?;
    state.values.set(key, value, ttl)?;
    if accepted && previous.is_none_or(|expiry| expiry < now) {
        let expiry = now.mul_add(1.0_f64, ttl.unwrap_or(settings.limits.default_ttl));
        if !expiry.is_finite() {
            return Err(BackendError::InvalidExpiry {
                key: key.to_owned(),
            });
        }
        state.expiries.insert(key.to_owned(), expiry);
    }
    Ok(())
}

// The shared cache decides admission from private state, so a one-entry copy applies its own size and key policy
fn accepts(settings: &MemorySettings, key: &str, value: &CacheValue) -> Result<bool, BackendError> {
    let item_kib = settings.construction.item_kib;
    let mut probe = LocalTtlCache::with_clock(
        Some(1),
        Some(600.0_f64),
        Some(item_kib),
        item_kib,
        Arc::clone(&settings.clock),
    )?;
    probe.set(key, value.clone(), None)?;
    Ok(probe.get(key)?.is_some())
}

// LiteLLM drops the entries with the earliest expiry once the cache is full, before it checks the new size
fn evict_to_capacity(expiries: &mut BTreeMap<String, f64>, capacity: usize) {
    if capacity == 0 {
        return;
    }
    while expiries.len() >= capacity {
        let victim = expiries
            .iter()
            .min_by(|left, right| left.1.total_cmp(right.1).then_with(|| left.0.cmp(right.0)))
            .map(|(key, _)| key.clone());
        let Some(earliest) = victim else { return };
        expiries.remove(&earliest);
    }
}

pub(super) fn increment(
    state: &mut MemoryState,
    settings: &MemorySettings,
    key: &str,
    value: f64,
    ttl: Option<f64>,
) -> Result<f64, BackendError> {
    let current = read(state, key)?;
    let base = match current {
        None => 0.0_f64,
        Some(CacheValue::Integer(number)) => {
            Number::from(number)
                .as_f64()
                .ok_or_else(|| BackendError::NotNumeric {
                    key: key.to_owned(),
                })?
        }
        Some(CacheValue::Float(number)) => number,
        Some(_) => {
            return Err(BackendError::NotNumeric {
                key: key.to_owned(),
            });
        }
    };
    let next = base.mul_add(1.0_f64, value);
    if !next.is_finite() {
        return Err(BackendError::IncrementOutOfRange {
            key: key.to_owned(),
        });
    }
    write(state, settings, key, CacheValue::Float(next), ttl)?;
    Ok(next)
}

fn read_clock(clock: &Clock) -> Result<f64, BackendError> {
    let now = clock();
    if now.is_finite() {
        Ok(now)
    } else {
        Err(BackendError::from(CacheError::InvalidClock))
    }
}

pub(super) fn whole_seconds(seconds: f64) -> Result<i64, BackendError> {
    let magnitude = Duration::try_from_secs_f64(seconds.abs()).map_err(|source| {
        BackendError::InvalidDefaultDuration {
            value: seconds,
            source,
        }
    })?;
    let whole = i64::try_from(magnitude.as_secs()).map_err(|source| {
        BackendError::InvalidDefaultSeconds {
            value: seconds,
            source,
        }
    })?;
    if seconds.is_sign_negative() {
        0_i64
            .checked_sub(whole)
            .ok_or(BackendError::InvalidDefaultTtl { value: seconds })
    } else {
        Ok(whole)
    }
}
