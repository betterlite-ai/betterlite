use std::collections::{BTreeMap, BTreeSet};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Mutex, PoisonError};

use secrecy::{ExposeSecret as _, SecretString};
use sha2::{Digest as _, Sha256};
use thiserror::Error;

use crate::local_ttl_cache::{CacheError, CacheValue, LocalTtlCache};

pub const DEFAULT_TTL_FOR_HTTPX_CLIENTS: f64 = 3600.0;

const ITEM_KIB: i64 = 1024;

pub type Clock = Arc<dyn Fn() -> f64 + Send + Sync>;

pub type RemovalEffect<Value> = Arc<dyn Fn(Value) + Send + Sync>;

#[derive(Debug, Error)]
pub enum DynamicLoggingCacheError {
    #[error("the logging client cache rejected the entry")]
    Cache {
        #[from]
        source: CacheError,
    },
}

struct Inner<Value> {
    markers: LocalTtlCache,
    values: BTreeMap<String, Value>,
}

pub struct DynamicLoggingCache<Value> {
    on_remove: RemovalEffect<Value>,
    inner: Mutex<Inner<Value>>,
}

fn push_field(canonical: &mut Vec<u8>, value: &[u8]) {
    canonical.extend_from_slice(&value.len().to_be_bytes());
    canonical.extend_from_slice(value);
}

// LiteLLM merges the service name over any credential of that name, so that credential never reaches the hash
fn cache_key(credentials: &BTreeMap<String, SecretString>, service_name: &str) -> String {
    let mut canonical = Vec::new();
    for (name, secret) in credentials {
        if name == "service_name" {
            continue;
        }
        push_field(&mut canonical, name.as_bytes());
        push_field(&mut canonical, secret.expose_secret().as_bytes());
    }
    push_field(&mut canonical, service_name.as_bytes());
    let mut hasher = Sha256::new();
    hasher.update(&canonical);
    hex::encode(hasher.finalize())
}

fn stop_logging_client<Value>(removal: &RemovalEffect<Value>, evicted: Value) {
    let effect = Arc::clone(removal);
    let stopped = catch_unwind(AssertUnwindSafe(move || effect(evicted)));
    if stopped.is_err() {
        tracing::debug!("dynamic logging cache: stop() failed during eviction");
    }
}

impl<Value: Clone + Send + 'static> DynamicLoggingCache<Value> {
    pub fn new(
        on_remove: RemovalEffect<Value>,
        clock: Clock,
    ) -> Result<Self, DynamicLoggingCacheError> {
        let markers = LocalTtlCache::with_clock(
            None,
            Some(DEFAULT_TTL_FOR_HTTPX_CLIENTS),
            None,
            ITEM_KIB,
            clock,
        )?;
        Ok(Self {
            on_remove,
            inner: Mutex::new(Inner {
                markers,
                values: BTreeMap::new(),
            }),
        })
    }

    pub fn get_cache(
        &self,
        credentials: &BTreeMap<String, SecretString>,
        service_name: &str,
    ) -> Result<Option<Value>, DynamicLoggingCacheError> {
        let key = cache_key(credentials, service_name);
        let mut inner = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        let expired = inner.markers.get(&key)?.is_none();
        let cached = inner.values.get(&key).cloned();
        let removed = if expired {
            inner.values.remove(&key)
        } else {
            None
        };
        drop(inner);
        if let Some(evicted) = removed {
            stop_logging_client(&self.on_remove, evicted);
        }
        Ok(if expired { None } else { cached })
    }

    pub fn set_cache(
        &self,
        credentials: &BTreeMap<String, SecretString>,
        service_name: &str,
        logging_client: Value,
    ) -> Result<(), DynamicLoggingCacheError> {
        let key = cache_key(credentials, service_name);
        let mut inner = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        let previous: BTreeSet<String> = inner
            .markers
            .ttl_entries()
            .into_iter()
            .map(|(marker, _expiry)| marker)
            .collect();
        inner.markers.set(&key, CacheValue::Null, None)?;
        let current: BTreeSet<String> = inner
            .markers
            .ttl_entries()
            .into_iter()
            .map(|(marker, _expiry)| marker)
            .collect();
        let mut stopped = Vec::new();
        for evicted_key in previous.difference(&current) {
            if let Some(evicted) = inner.values.remove(evicted_key) {
                stopped.push(evicted);
            }
        }
        if let Some(replaced) = inner.values.insert(key, logging_client) {
            stopped.push(replaced);
        }
        drop(inner);
        for evicted in stopped {
            stop_logging_client(&self.on_remove, evicted);
        }
        Ok(())
    }
}
