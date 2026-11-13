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
    let stopped = catch_unwind(AssertUnwindSafe(|| removal(evicted)));
    if stopped.is_err() {
        tracing::debug!("dynamic logging cache: stop() failed during eviction");
    }
}

// the shared TTL-marker storage: LocalTtlCache owns expiry and capacity for values it cannot hold
pub fn store_marked<Value>(
    markers: &mut LocalTtlCache,
    values: &mut BTreeMap<String, Value>,
    key: String,
    value: Value,
    ttl: Option<f64>,
) -> Result<(Vec<Value>, Option<Value>), CacheError> {
    let previous: BTreeSet<String> = markers
        .ttl_entries()
        .into_iter()
        .map(|(marker, _expiry)| marker)
        .collect();
    markers.set(&key, CacheValue::Null, ttl)?;
    let current: BTreeSet<String> = markers
        .ttl_entries()
        .into_iter()
        .map(|(marker, _expiry)| marker)
        .collect();
    let mut evicted = Vec::new();
    for evicted_key in previous.difference(&current) {
        if let Some(evicted_value) = values.remove(evicted_key) {
            evicted.push(evicted_value);
        }
    }
    let replaced = if current.contains(&key) {
        values.insert(key, value)
    } else {
        None
    };
    Ok((evicted, replaced))
}

// the value still live under the key, then the one whose marker has just expired
pub fn lookup_marked<Value>(
    markers: &mut LocalTtlCache,
    values: &mut BTreeMap<String, Value>,
    key: &str,
) -> Result<(Option<Value>, Option<Value>), CacheError>
where
    Value: Clone,
{
    let present = markers.get(key)?.is_some();
    let cached = values.get(key).cloned();
    let expired = if present { None } else { values.remove(key) };
    Ok((if present { cached } else { None }, expired))
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
        let mut guard = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        let inner = &mut *guard;
        let (cached, expired) = lookup_marked(&mut inner.markers, &mut inner.values, &key)?;
        drop(guard);
        if let Some(evicted) = expired {
            stop_logging_client(&self.on_remove, evicted);
        }
        Ok(cached)
    }

    pub fn set_cache(
        &self,
        credentials: &BTreeMap<String, SecretString>,
        service_name: &str,
        logging_client: Value,
    ) -> Result<(), DynamicLoggingCacheError> {
        let key = cache_key(credentials, service_name);
        let mut guard = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        let inner = &mut *guard;
        let (evicted, replaced) = store_marked(
            &mut inner.markers,
            &mut inner.values,
            key,
            logging_client,
            None,
        )?;
        drop(guard);
        let mut stopped = evicted;
        if let Some(replaced_client) = replaced {
            stopped.push(replaced_client);
        }
        for evicted_client in stopped {
            stop_logging_client(&self.on_remove, evicted_client);
        }
        Ok(())
    }
}
