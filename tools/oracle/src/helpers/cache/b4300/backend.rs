use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_cache::backend::{CacheResult, Ttl};
use betterlite_cache::envelope::backend::EnvelopeBackend;
use betterlite_cache::envelope::lookup::CacheLookup;
use betterlite_context::local_ttl_cache::CacheValue;

#[derive(Default)]
pub(crate) struct SeededBackend {
    stored: Mutex<BTreeMap<String, CacheValue>>,
}

impl SeededBackend {
    pub(crate) fn seed(&self, key: &str, value: CacheValue) {
        self.stored
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key.to_owned(), value);
    }

    fn read(&self, key: &str) -> Option<CacheValue> {
        self.stored
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(key)
            .cloned()
    }
}

#[async_trait]
impl EnvelopeBackend for SeededBackend {
    fn get_cache(&self, key: &str, _lookup: &mut CacheLookup) -> CacheResult<Option<CacheValue>> {
        Ok(self.read(key))
    }

    fn set_cache(&self, key: &str, value: CacheValue, _ttl: Ttl) -> CacheResult<()> {
        self.seed(key, value);
        Ok(())
    }

    async fn async_get_cache(
        &self,
        key: &str,
        _lookup: &mut CacheLookup,
    ) -> CacheResult<Option<CacheValue>> {
        Ok(self.read(key))
    }

    async fn async_set_cache(&self, key: &str, value: CacheValue, _ttl: Ttl) -> CacheResult<()> {
        self.seed(key, value);
        Ok(())
    }

    async fn batch_cache_write(&self, key: &str, value: CacheValue, _ttl: Ttl) -> CacheResult<()> {
        self.seed(key, value);
        Ok(())
    }
}
