use std::collections::BTreeSet;

use crate::limiter_records::{
    AtomicCounterMeta, AtomicCounterState, CacheCounterValue, RateLimitResponse, RateLimitStatus,
    ReservationBackend, ReservationWindowIdentity, sibling_counter_keys,
};

use super::rate_limit_kind::status_rate_limit_type;
use super::value::{CounterValueError, counter_integer};

mod tests;

#[async_trait::async_trait]
pub trait LocalCounterCache: Send + Sync {
    async fn get(&self, key: &str, local_only: bool) -> Option<CacheCounterValue>;

    async fn set(&self, key: &str, value: CacheCounterValue, ttl: i64, local_only: bool);
}

// ast-grep-ignore: no-long-names
pub async fn atomic_check_and_increment_in_memory(
    cache: &dyn LocalCounterCache,
    per_counter_meta: &[AtomicCounterMeta],
    now: i64,
) -> Result<RateLimitResponse, CounterValueError> {
    let mut states = Vec::with_capacity(per_counter_meta.len());
    for meta in per_counter_meta {
        let window_start = match cache.get(&meta.window_key, true).await {
            None => None,
            Some(value) => Some(counter_integer(&meta.window_key, &value)?),
        };
        let window_expired =
            window_start.is_none_or(|start| now.saturating_sub(start) >= meta.window_size);
        let current = if window_expired {
            0
        } else {
            match cache.get(&meta.counter_key, true).await {
                None => 0,
                Some(value) => counter_integer(&meta.counter_key, &value)?,
            }
        };
        if over_limit(meta, current) {
            return Ok(over_limit_response(meta, current));
        }
        states.push(AtomicCounterState {
            window_expired,
            current,
            window_start: match window_start {
                Some(start) if !window_expired => start.to_string(),
                _ => now.to_string(),
            },
        });
    }
    Ok(apply_increments(cache, per_counter_meta, &states, now).await)
}

fn over_limit(meta: &AtomicCounterMeta, current: i64) -> bool {
    if meta.increment > 0 {
        current.saturating_add(meta.increment) > meta.current_limit
    } else {
        current >= meta.current_limit
    }
}

fn over_limit_response(meta: &AtomicCounterMeta, current: i64) -> RateLimitResponse {
    RateLimitResponse {
        overall_code: "OVER_LIMIT".to_owned(),
        statuses: vec![RateLimitStatus {
            code: "OVER_LIMIT".to_owned(),
            current_limit: meta.current_limit,
            limit_remaining: meta.current_limit.saturating_sub(current).max(0),
            rate_limit_type: status_rate_limit_type(meta.rate_limit_type),
            descriptor_key: meta.descriptor_key.clone(),
            descriptor_value: Some(meta.descriptor_value.clone()),
        }],
        reservation_windows: None,
    }
}

async fn reset_siblings(
    cache: &dyn LocalCounterCache,
    per_counter_meta: &[AtomicCounterMeta],
    states: &[AtomicCounterState],
) {
    let mut expired_windows: Vec<(&str, i64)> = Vec::new();
    for (meta, state) in per_counter_meta.iter().zip(states) {
        let already_reset = expired_windows
            .iter()
            .any(|(window_key, _)| *window_key == meta.window_key.as_str());
        if state.window_expired && !already_reset {
            expired_windows.push((meta.window_key.as_str(), meta.window_size));
        }
    }
    for (window_key, window_size) in expired_windows {
        let (requests_key, tokens_key) = sibling_counter_keys(window_key);
        cache
            .set(&requests_key, CacheCounterValue::Int(0), window_size, true)
            .await;
        cache
            .set(&tokens_key, CacheCounterValue::Int(0), window_size, true)
            .await;
    }
}

async fn apply_increments(
    cache: &dyn LocalCounterCache,
    per_counter_meta: &[AtomicCounterMeta],
    states: &[AtomicCounterState],
    now: i64,
) -> RateLimitResponse {
    reset_siblings(cache, per_counter_meta, states).await;
    let mut statuses = Vec::with_capacity(per_counter_meta.len());
    let mut windows = BTreeSet::new();
    for (meta, state) in per_counter_meta.iter().zip(states) {
        let new_counter = if state.window_expired {
            meta.increment
        } else {
            state.current.saturating_add(meta.increment)
        };
        if state.window_expired {
            cache
                .set(
                    &meta.window_key,
                    CacheCounterValue::Text(now.to_string()),
                    meta.window_size,
                    true,
                )
                .await;
        }
        cache
            .set(
                &meta.counter_key,
                CacheCounterValue::Int(new_counter),
                meta.ttl,
                true,
            )
            .await;
        statuses.push(RateLimitStatus {
            code: "OK".to_owned(),
            current_limit: meta.current_limit,
            limit_remaining: meta.current_limit.saturating_sub(new_counter).max(0),
            rate_limit_type: status_rate_limit_type(meta.rate_limit_type),
            descriptor_key: meta.descriptor_key.clone(),
            descriptor_value: Some(meta.descriptor_value.clone()),
        });
        windows.insert(ReservationWindowIdentity {
            counter_key: meta.counter_key.clone(),
            window_start: state.window_start.clone(),
            backend: ReservationBackend::Local,
        });
    }
    RateLimitResponse {
        overall_code: "OK".to_owned(),
        statuses,
        reservation_windows: Some(windows),
    }
}
