use std::collections::BTreeMap;
use std::sync::Mutex;

use async_trait::async_trait;
use betterlite_core::api::responses_poll_state::cache::{PollingCache, PollingCacheError};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::sdk_http::error::SdkHttpError;

const POISONED: &str = "the capture cache lock is poisoned";
const STATUS_CANCELLED: &str = "cancelled";
const STATUS_FIELD: &str = "status";

#[derive(Debug, Default)]
struct State {
    entries: BTreeMap<String, String>,
    writes: Vec<Json>,
}

// the RedisCache stand-in: it holds the seeded record, records every write and can play the cancel route
pub(super) struct CaptureCache {
    state: Mutex<State>,
    cancel_after_write: Option<usize>,
}

impl CaptureCache {
    pub(super) fn new(
        seed_key: String,
        seed: &Json,
        cancel_after_write: Option<usize>,
    ) -> Result<Self, SdkHttpError> {
        let encoded = serde_json::to_string(seed).map_err(SdkHttpError::Serialize)?;
        let mut state = State::default();
        state.entries.insert(seed_key, encoded);
        Ok(Self {
            state: Mutex::new(state),
            cancel_after_write,
        })
    }

    pub(super) fn writes(&self) -> Result<Vec<Json>, SdkHttpError> {
        let state = self
            .state
            .lock()
            .map_err(|_error| SdkHttpError::Invalid(POISONED.to_owned()))?;
        Ok(state.writes.clone())
    }
}

#[async_trait]
impl PollingCache for CaptureCache {
    async fn read(&self, key: &str) -> Result<Option<Vec<u8>>, PollingCacheError> {
        let state = self
            .state
            .lock()
            .map_err(|_error| PollingCacheError::Read(POISONED.to_owned()))?;
        Ok(state
            .entries
            .get(key)
            .map(|value| value.as_bytes().to_vec()))
    }

    async fn write(&self, key: &str, value: &str, _ttl: i64) -> Result<(), PollingCacheError> {
        let parsed = serde_json::from_str::<Json>(value)
            .map_err(|error| PollingCacheError::Write(error.to_string()))?;
        let mut state = self
            .state
            .lock()
            .map_err(|_error| PollingCacheError::Write(POISONED.to_owned()))?;
        state.entries.insert(key.to_owned(), value.to_owned());
        state.writes.push(parsed);
        if self.cancel_after_write == Some(state.writes.len()) {
            cancel(&mut state, key)?;
        }
        Ok(())
    }

    async fn delete(&self, key: &str) -> Result<i64, PollingCacheError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_error| PollingCacheError::Delete(POISONED.to_owned()))?;
        Ok(i64::from(state.entries.remove(key).is_some()))
    }
}

// the cancel route writes the same key from outside the worker, and the worker never reads it back
fn cancel(state: &mut State, key: &str) -> Result<(), PollingCacheError> {
    let stored = state
        .entries
        .get(key)
        .cloned()
        .ok_or_else(|| PollingCacheError::Write(POISONED.to_owned()))?;
    let mut response: IndexMap<String, Json> = serde_json::from_str(&stored)
        .map_err(|error| PollingCacheError::Write(error.to_string()))?;
    response.insert(
        STATUS_FIELD.to_owned(),
        Json::Text(STATUS_CANCELLED.to_owned()),
    );
    let encoded = serde_json::to_string(&Json::Object(response))
        .map_err(|error| PollingCacheError::Write(error.to_string()))?;
    state.entries.insert(key.to_owned(), encoded);
    Ok(())
}
