use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_context::local_ttl_cache::CacheValue;
use betterlite_server::mcp::outbound_credentials::dual_cache::AsyncCache;
use betterlite_server::mcp::outbound_credentials::v2_store::{CredentialPayload, CredentialReader};
use betterlite_server::mcp::outbound_credentials::{
    OAuthToken, OAuthTokenStore, TokenRefresher, TokenStoreUnavailable, WallClock,
};
use chrono::{DateTime, Utc};
use serde_json::Value;
use thiserror::Error;

use super::{StoreResult, outcomes, value_error};
use crate::helpers::HelperError;

#[derive(Debug)]
pub(super) struct ManualClock(Mutex<DateTime<Utc>>);

impl Default for ManualClock {
    fn default() -> Self {
        Self(Mutex::new(DateTime::<Utc>::UNIX_EPOCH))
    }
}

impl ManualClock {
    pub(super) fn set_millis(&self, millis: i64) -> Result<(), Box<HelperError>> {
        let instant = DateTime::from_timestamp_millis(millis)
            .ok_or_else(|| value_error(format!("invalid epoch millis: {millis}")))?;
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = instant;
        Ok(())
    }
}

impl WallClock for ManualClock {
    fn now(&self) -> DateTime<Utc> {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

pub(super) struct ScriptedStore {
    results: Mutex<VecDeque<StoreResult>>,
    pub(super) calls: AtomicUsize,
}

impl ScriptedStore {
    pub(super) fn new(value: Option<&Value>) -> Result<Self, Box<HelperError>> {
        Ok(Self {
            results: Mutex::new(outcomes(value)?),
            calls: AtomicUsize::new(0),
        })
    }

    fn next(&self) -> StoreResult {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.results
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap_or_else(|| {
                Err(TokenStoreUnavailable {
                    detail: "out of results".to_owned(),
                })
            })
    }
}

#[async_trait]
impl OAuthTokenStore for ScriptedStore {
    async fn fetch(&self, _user_id: &str, _server_id: &str) -> StoreResult {
        self.next()
    }
}

#[async_trait]
impl TokenRefresher for ScriptedStore {
    async fn refresh(&self, _user_id: &str, _server_id: &str, _token: &OAuthToken) -> StoreResult {
        self.next()
    }
}

#[derive(Debug, Error)]
#[error("the cache is unavailable")]
pub(super) struct CacheUnavailable;

pub(super) struct FakeCache {
    pub(super) get: Option<String>,
    pub(super) set_error: bool,
    pub(super) delete_error: bool,
    pub(super) stored: Mutex<BTreeMap<String, String>>,
}

#[async_trait]
impl AsyncCache for FakeCache {
    type Error = CacheUnavailable;

    async fn async_get_cache(&self, key: &str) -> Result<Option<CacheValue>, CacheUnavailable> {
        if self.get.as_deref() == Some("error") {
            return Err(CacheUnavailable);
        }
        if self.get.as_deref() != Some("stored") {
            return Ok(None);
        }
        let stored = self
            .stored
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(key)
            .cloned();
        Ok(stored.map(CacheValue::Text))
    }

    async fn async_set_cache(
        &self,
        key: &str,
        value: &str,
        _ttl: Option<Duration>,
    ) -> Result<(), CacheUnavailable> {
        if self.set_error {
            return Err(CacheUnavailable);
        }
        self.stored
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key.to_owned(), value.to_owned());
        Ok(())
    }

    async fn async_delete_cache(&self, key: &str) -> Result<(), CacheUnavailable> {
        if self.delete_error {
            return Err(CacheUnavailable);
        }
        self.stored
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(key);
        Ok(())
    }
}

pub(super) struct PayloadReader {
    pub(super) payloads: Mutex<VecDeque<Option<CredentialPayload>>>,
}

impl PayloadReader {
    pub(super) fn new(value: Option<&Value>) -> Self {
        let payloads = value
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .map(|item| serde_json::from_value(item.clone()).ok().flatten())
                    .collect()
            })
            .unwrap_or_default();
        Self {
            payloads: Mutex::new(payloads),
        }
    }
}

#[async_trait]
impl CredentialReader for PayloadReader {
    async fn read_credential(
        &self,
        _user_id: &str,
        _server_id: &str,
    ) -> Result<Option<CredentialPayload>, TokenStoreUnavailable> {
        Ok(self
            .payloads
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .flatten())
    }
}
