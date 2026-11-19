use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_router::model_rate_limits::cache::{RateLimitCache, RemoteRead};
use betterlite_router::model_rate_limits::keys::RateKeys;

use super::{Counters, Payload, RedisMode};

// The DualCache the check counts in: an in-memory tier that answers synchronously and a fake Redis behind the selected failure mode
pub(super) struct FakeCache {
    local: Mutex<BTreeMap<String, i64>>,
    remote: Mutex<BTreeMap<String, i64>>,
    pub(super) mode: Mutex<RedisMode>,
    has_remote: bool,
}

impl FakeCache {
    pub(super) fn new(payload: &Payload, keys: &RateKeys) -> Self {
        let cache = Self {
            local: Mutex::new(BTreeMap::new()),
            remote: Mutex::new(BTreeMap::new()),
            mode: Mutex::new(payload.redis),
            has_remote: payload.redis != RedisMode::None,
        };
        seed(&cache.local, keys, payload.local);
        seed(&cache.remote, keys, payload.remote);
        cache
    }
}

fn seed(tier: &Mutex<BTreeMap<String, i64>>, keys: &RateKeys, counters: Counters) {
    let mut values = tier.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(value) = counters.tpm {
        values.insert(keys.tpm.clone(), value);
    }
    if let Some(value) = counters.rpm {
        values.insert(keys.rpm.clone(), value);
    }
}

#[async_trait]
impl RateLimitCache for FakeCache {
    fn local(&self, key: &str) -> Option<i64> {
        self.local
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(key)
            .copied()
    }

    fn has_remote(&self) -> bool {
        self.has_remote
    }

    async fn remote(&self, key: &str) -> RemoteRead {
        if !self.has_remote {
            return RemoteRead::Value(None);
        }
        let mode = *self.mode.lock().unwrap_or_else(PoisonError::into_inner);
        match mode {
            RedisMode::Up => RemoteRead::Value(
                self.remote
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .get(key)
                    .copied(),
            ),
            // RedisCache.async_get_cache swallows a connection failure and answers None, so a down store reads as absent
            RedisMode::Down | RedisMode::None => RemoteRead::Value(None),
            RedisMode::Breaker => RemoteRead::BreakerOpen,
        }
    }

    fn increment_local(&self, key: &str, by: i64, _ttl_seconds: u64) -> Option<i64> {
        let mut values = self.local.lock().unwrap_or_else(PoisonError::into_inner);
        let next = values
            .get(key)
            .copied()
            .unwrap_or_default()
            .saturating_add(by);
        values.insert(key.to_owned(), next);
        drop(values);
        Some(next)
    }

    async fn increment(&self, key: &str, by: i64, ttl_seconds: u64) -> Option<i64> {
        let local = self.increment_local(key, by, ttl_seconds);
        let mode = *self.mode.lock().unwrap_or_else(PoisonError::into_inner);
        if !self.has_remote || mode != RedisMode::Up {
            return local;
        }
        let mut values = self.remote.lock().unwrap_or_else(PoisonError::into_inner);
        let next = values
            .get(key)
            .copied()
            .unwrap_or_default()
            .saturating_add(by);
        values.insert(key.to_owned(), next);
        drop(values);
        Some(next)
    }
}
