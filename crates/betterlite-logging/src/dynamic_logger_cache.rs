use std::collections::BTreeMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Mutex};

use secrecy::{ExposeSecret as _, SecretString};
use sha2::{Digest as _, Sha256};
use thiserror::Error;

pub const DEFAULT_TTL_FOR_HTTPX_CLIENTS: f64 = 3600.0;

pub type RemovalEffect<V> = Arc<dyn Fn(V) + Send + Sync>;

#[derive(Debug, Error)]
pub enum DynamicLoggerCacheError {
    #[error("dynamic logger cache lock is poisoned")]
    Unavailable,
    #[error("dynamic logger cache clock returned a non-finite time")]
    InvalidClock,
    #[error("dynamic logger cache key failed to serialize")]
    Serialize {
        #[source]
        source: serde_json::Error,
    },
}

pub struct Entry<V> {
    pub value: V,
    pub expiry: f64,
}

pub struct DynamicLoggerCache<V> {
    pub default_ttl: f64,
    pub clock: Arc<dyn Fn() -> f64 + Send + Sync>,
    pub on_remove: RemovalEffect<V>,
    pub entries: Mutex<BTreeMap<String, Entry<V>>>,
}

pub fn get_cache_key(
    credentials: &BTreeMap<String, SecretString>,
    service_name: &str,
) -> Result<String, DynamicLoggerCacheError> {
    let mut keyed: BTreeMap<&str, &str> = credentials
        .iter()
        .map(|(key, value)| (key.as_str(), value.expose_secret()))
        .collect();
    keyed.insert("service_name", service_name);
    let serialized = serde_json::to_string(&keyed)
        .map_err(|source| DynamicLoggerCacheError::Serialize { source })?;
    let mut hasher = Sha256::new();
    hasher.update(serialized.as_bytes());
    Ok(hex::encode(hasher.finalize()))
}

impl<V: Clone> DynamicLoggerCache<V> {
    pub fn get_cache(
        &self,
        credentials: &BTreeMap<String, SecretString>,
        service_name: &str,
    ) -> Result<Option<V>, DynamicLoggerCacheError> {
        let key = get_cache_key(credentials, service_name)?;
        let now = self.read_clock()?;
        let mut entries = self
            .entries
            .lock()
            .map_err(|_poisoned| DynamicLoggerCacheError::Unavailable)?;
        let Some(entry) = entries.get(&key) else {
            return Ok(None);
        };
        if entry.expiry >= now {
            return Ok(Some(entry.value.clone()));
        }
        let removed = entries.remove(&key);
        drop(entries);
        if let Some(removed_entry) = removed {
            self.stop(removed_entry.value);
        }
        Ok(None)
    }

    // Overwrite always replaces the value, but keeps a still-live expiry (allow_ttl_override).
    pub fn set_cache(
        &self,
        credentials: &BTreeMap<String, SecretString>,
        service_name: &str,
        value: V,
    ) -> Result<(), DynamicLoggerCacheError> {
        let key = get_cache_key(credentials, service_name)?;
        let now = self.read_clock()?;
        let mut entries = self
            .entries
            .lock()
            .map_err(|_poisoned| DynamicLoggerCacheError::Unavailable)?;
        let fresh_expiry = now.mul_add(1.0_f64, self.default_ttl);
        let expiry = match entries.get(&key) {
            Some(entry) if entry.expiry >= now => entry.expiry,
            _ => fresh_expiry,
        };
        entries.insert(key, Entry { value, expiry });
        drop(entries);
        Ok(())
    }

    pub fn evict(
        &self,
        credentials: &BTreeMap<String, SecretString>,
        service_name: &str,
    ) -> Result<(), DynamicLoggerCacheError> {
        let key = get_cache_key(credentials, service_name)?;
        let removed = {
            let mut entries = self
                .entries
                .lock()
                .map_err(|_poisoned| DynamicLoggerCacheError::Unavailable)?;
            entries.remove(&key)
        };
        if let Some(entry) = removed {
            self.stop(entry.value);
        }
        Ok(())
    }

    fn read_clock(&self) -> Result<f64, DynamicLoggerCacheError> {
        let now = (self.clock)();
        if now.is_finite() {
            Ok(now)
        } else {
            Err(DynamicLoggerCacheError::InvalidClock)
        }
    }

    // Mirrors LangfuseInMemoryCache._remove_key's try/except: a failing stop() stays local.
    fn stop(&self, value: V) {
        let effect = Arc::clone(&self.on_remove);
        if catch_unwind(AssertUnwindSafe(move || effect(value))).is_err() {
            tracing::debug!("dynamic logger cache: stop() panicked during eviction");
        }
    }
}

mod tests;
