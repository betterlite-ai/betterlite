use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_core::api::responses_poll_state::cache::{PollingCache, PollingCacheError};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::sdk_http::error::SdkHttpError;

// the RedisCache stand-in: it holds the seeded record, records every write and can play the cancel route
pub(super) struct CaptureCache {
    entries: Mutex<BTreeMap<String, String>>,
    writes: Mutex<Vec<Json>>,
    cancel_after_write: Option<usize>,
}

impl CaptureCache {
    pub(super) fn new(
        seed_key: String,
        seed: &Json,
        cancel_after_write: Option<usize>,
    ) -> Result<Self, SdkHttpError> {
        let encoded = serde_json::to_string(seed).map_err(SdkHttpError::Serialize)?;
        Ok(Self {
            entries: Mutex::new(BTreeMap::from([(seed_key, encoded)])),
            writes: Mutex::new(Vec::new()),
            cancel_after_write,
        })
    }

    pub(super) fn writes(&self) -> Vec<Json> {
        self.writes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl PollingCache for CaptureCache {
    async fn read(&self, key: &str) -> Result<Option<Vec<u8>>, PollingCacheError> {
        let entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        Ok(entries.get(key).map(|value| value.as_bytes().to_vec()))
    }

    async fn write(&self, key: &str, value: &str, _ttl: i64) -> Result<(), PollingCacheError> {
        let parsed = serde_json::from_str::<Json>(value)
            .map_err(|error| PollingCacheError::Write(error.to_string()))?;
        let count = {
            let mut writes = self.writes.lock().unwrap_or_else(PoisonError::into_inner);
            writes.push(parsed);
            writes.len()
        };
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key.to_owned(), value.to_owned());
        if self.cancel_after_write == Some(count) {
            cancel(self, key)?;
        }
        Ok(())
    }

    async fn delete(&self, key: &str) -> Result<i64, PollingCacheError> {
        let mut entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        Ok(i64::from(entries.remove(key).is_some()))
    }
}

// the cancel route writes the same key from outside the worker, and the worker never reads it back
fn cancel(cache: &CaptureCache, key: &str) -> Result<(), PollingCacheError> {
    let entries = cache.entries.lock().unwrap_or_else(PoisonError::into_inner);
    let Some(stored) = entries.get(key).cloned() else {
        return Ok(());
    };
    drop(entries);
    let mut response: IndexMap<String, Json> = serde_json::from_str(&stored)
        .map_err(|error| PollingCacheError::Write(error.to_string()))?;
    response.insert("status".to_owned(), Json::Text("cancelled".to_owned()));
    let encoded = serde_json::to_string(&Json::Object(response))
        .map_err(|error| PollingCacheError::Write(error.to_string()))?;
    let mut written = cache.entries.lock().unwrap_or_else(PoisonError::into_inner);
    written.insert(key.to_owned(), encoded);
    drop(written);
    Ok(())
}
