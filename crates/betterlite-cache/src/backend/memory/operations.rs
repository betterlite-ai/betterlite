use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_context::local_ttl_cache::{CacheError, CacheValue};
use serde_json::Number;

use super::{Clock, MemorySettings, MemoryState};
use crate::backend::BackendError;

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
    // LiteLLM prunes expired entries and drops the earliest expiry before it checks the new size
    state.expiries.retain(|_, expiry| *expiry > now);
    if let Some(victim) = eviction_victim(&state.expiries, settings.limits.capacity) {
        state.expiries.remove(&victim);
    }
    let kept = state.expiries.contains_key(key);
    state.values.set(key, value, ttl)?;
    if !kept && state.values.get(key)?.is_some() {
        let expiry = now.mul_add(1.0_f64, ttl.unwrap_or(settings.limits.default_ttl));
        state.expiries.insert(key.to_owned(), expiry);
    }
    Ok(())
}

// LiteLLM's set drops the one entry with the earliest expiry when the cache is already full
fn eviction_victim(expiries: &BTreeMap<String, f64>, capacity: usize) -> Option<String> {
    if capacity == 0 || expiries.len() < capacity {
        return None;
    }
    expiries
        .iter()
        .min_by(|left, right| left.1.total_cmp(right.1).then_with(|| left.0.cmp(right.0)))
        .map(|(key, _)| key.clone())
}

pub(super) fn increment(
    state: &mut MemoryState,
    settings: &MemorySettings,
    key: &str,
    value: f64,
    ttl: Option<f64>,
) -> Result<f64, BackendError> {
    let base = stored_number(state, key)?;
    let next = base.mul_add(1.0_f64, value);
    if !next.is_finite() {
        return Err(BackendError::IncrementOutOfRange {
            key: key.to_owned(),
        });
    }
    write(state, settings, key, CacheValue::Float(next), ttl)?;
    Ok(next)
}

fn stored_number(state: &mut MemoryState, key: &str) -> Result<f64, BackendError> {
    match read(state, key)? {
        None => Ok(0.0_f64),
        Some(CacheValue::Integer(number)) => {
            Number::from(number)
                .as_f64()
                .ok_or_else(|| BackendError::NotNumeric {
                    key: key.to_owned(),
                })
        }
        Some(CacheValue::Float(number)) => Ok(number),
        Some(_) => Err(BackendError::NotNumeric {
            key: key.to_owned(),
        }),
    }
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
