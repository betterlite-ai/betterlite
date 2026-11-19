use std::sync::{Mutex, PoisonError};

use betterlite_context::local_ttl_cache::{CacheError, CacheValue, LocalTtlCache};
use betterlite_models::decisions::Json;

use super::error::AffinityError;
use super::identity::AffinityKey;
use super::pin::{AffinityPin, StoredPin};
use crate::routing_value_store::{Clock, StoreSettings};

// the prefix keeps the cached text away from LocalTtlCache's json.loads, which would turn a pin object into a map of cache values
const PIN_BYTES_PREFIX: &[u8] = &[0];

pub struct LocalPins {
    cache: Mutex<LocalTtlCache>,
}

impl LocalPins {
    pub fn new(settings: StoreSettings, clock: Clock) -> Result<Self, CacheError> {
        let cache = LocalTtlCache::with_clock(
            settings.max_size_in_memory,
            settings.default_ttl_seconds,
            settings.max_size_per_item,
            settings.loaded_max_size_per_item_kib,
            clock,
        )?;
        Ok(Self {
            cache: Mutex::new(cache),
        })
    }

    // set_local_affinity_pin replaces the entry, because InMemoryCache.set_cache preserves a live key's expiry
    pub fn set(
        &self,
        key: &AffinityKey,
        pin: &StoredPin,
        ttl_seconds: u32,
    ) -> Result<(), AffinityError> {
        let mut cache = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
        cache.delete(key.as_ref());
        if matches!(pin, StoredPin::Other(Json::Null)) {
            drop(cache);
            return Ok(());
        }
        let bytes = stored_bytes(pin);
        cache.set(
            key.as_ref(),
            CacheValue::Bytes(bytes),
            Some(f64::from(ttl_seconds)),
        )?;
        drop(cache);
        Ok(())
    }

    pub fn read(&self, key: &AffinityKey) -> Result<Option<StoredPin>, AffinityError> {
        let mut cache = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
        let read = cache.get(key.as_ref())?;
        drop(cache);
        match read {
            None => Ok(None),
            Some(CacheValue::Bytes(bytes)) => {
                let Some(text) = bytes.strip_prefix(PIN_BYTES_PREFIX) else {
                    return Err(AffinityError::InvalidLocalValue);
                };
                let stored = StoredPin::parse(text)?;
                Ok(Some(stored))
            }
            Some(
                CacheValue::Credential(_)
                | CacheValue::Null
                | CacheValue::Boolean(_)
                | CacheValue::Integer(_)
                | CacheValue::Float(_)
                | CacheValue::Text(_)
                | CacheValue::Array(_)
                | CacheValue::Object(_),
            ) => Err(AffinityError::InvalidLocalValue),
        }
    }

    // claim_affinity_pin_in_memory: the first writer wins, a matching legacy value is rewritten, and an ineligible winner is replaced
    pub fn claim_in_memory(
        &self,
        key: &AffinityKey,
        pin: &AffinityPin,
        ttl_seconds: u32,
        eligible: Option<&[AffinityPin]>,
    ) -> Result<StoredPin, AffinityError> {
        let existing = self.read(key)?;
        if let Some(existing) = &existing
            && eligible.is_none()
        {
            if existing.matches_legacy(pin) {
                self.set(key, &StoredPin::Pin(pin.clone()), ttl_seconds)?;
            }
            return Ok(existing.clone());
        }
        let winner = match &existing {
            Some(existing)
                if eligible.is_some_and(|candidates| {
                    candidates
                        .iter()
                        .any(|candidate| existing.equals_pin(candidate))
                }) =>
            {
                existing.clone()
            }
            Some(_existing) | None => StoredPin::Pin(pin.clone()),
        };
        self.set(key, &winner, ttl_seconds)?;
        Ok(winner)
    }

    #[cfg(any(test, feature = "testing"))]
    pub fn expiry_seconds(&self, key: &AffinityKey) -> Result<Option<f64>, AffinityError> {
        let cache = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
        let entries = cache.ttl_entries();
        drop(cache);
        let found = entries
            .into_iter()
            .find(|(stored, _expiry)| stored == key.as_ref());
        Ok(found.map(|(_stored, expiry)| expiry))
    }
}

fn stored_bytes(pin: &StoredPin) -> Vec<u8> {
    let text = pin.wire_text();
    let capacity = text.len().saturating_add(PIN_BYTES_PREFIX.len());
    let mut bytes = Vec::with_capacity(capacity);
    bytes.extend_from_slice(PIN_BYTES_PREFIX);
    bytes.extend_from_slice(text.as_bytes());
    bytes
}
