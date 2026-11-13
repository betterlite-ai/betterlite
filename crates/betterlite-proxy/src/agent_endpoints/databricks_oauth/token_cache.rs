use std::collections::BTreeMap;
use std::sync::{Arc, Mutex as StdMutex, PoisonError};

use betterlite_context::local_ttl_cache::credential::{Credential, CredentialHandle};
use betterlite_context::local_ttl_cache::{CacheError, CacheValue, LocalTtlCache};
use betterlite_http::proxy::ProxiedClient;
use secrecy::SecretString;
use tokio::sync::Mutex;

use super::config::DatabricksAppOAuthConfig;
use super::error::DatabricksOauthError;
use super::fetch::fetch_token;

const DEFAULT_TTL_SECONDS: f64 = 3600.0;
const MAX_SIZE_PER_ITEM_KIB: i64 = 1024;

type Clock = Arc<dyn Fn() -> f64 + Send + Sync>;
pub type KeyLock = Arc<Mutex<()>>;

struct State {
    tokens: LocalTtlCache,
    // one lock per live cache key, so concurrent callers share a single token request
    locks: BTreeMap<String, KeyLock>,
}

pub struct DatabricksAppOAuthTokenCache {
    client: ProxiedClient,
    clock: Clock,
    state: StdMutex<State>,
}

fn new_token_cache(clock: &Clock) -> Result<LocalTtlCache, CacheError> {
    LocalTtlCache::with_clock(
        None,
        Some(DEFAULT_TTL_SECONDS),
        None,
        MAX_SIZE_PER_ITEM_KIB,
        Arc::clone(clock),
    )
}

impl State {
    // The lock of a key that left the cache goes with it, unless a caller still waits on it.
    fn drop_idle_locks(&mut self) {
        self.locks.retain(|key, lock| {
            Arc::strong_count(lock) > 1 || self.tokens.get(key).is_ok_and(|found| found.is_some())
        });
    }
}

impl DatabricksAppOAuthTokenCache {
    pub fn new(client: ProxiedClient, clock: Clock) -> Result<Self, DatabricksOauthError> {
        let tokens = new_token_cache(&clock).map_err(DatabricksOauthError::Cache)?;
        Ok(Self {
            client,
            clock,
            state: StdMutex::new(State {
                tokens,
                locks: BTreeMap::new(),
            }),
        })
    }

    pub fn flush_cache(&self) -> Result<(), DatabricksOauthError> {
        let fresh = new_token_cache(&self.clock).map_err(DatabricksOauthError::Cache)?;
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.tokens = fresh;
        state.locks.clear();
        drop(state);
        Ok(())
    }

    pub async fn async_get_token(
        &self,
        config: &DatabricksAppOAuthConfig,
    ) -> Result<SecretString, DatabricksOauthError> {
        let cache_key = config.cache_key();
        if let Some(token) = self.cached_token(&cache_key)? {
            return Ok(token);
        }
        let lock = self.key_lock(&cache_key);
        let _coalescing = lock.lock().await;
        if let Some(token) = self.cached_token(&cache_key)? {
            return Ok(token);
        }
        let (token, ttl) = fetch_token(&self.client, config).await?;
        if ttl > 0 {
            self.store_token(&cache_key, &token, ttl)?;
        } else {
            let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            state.locks.remove(&cache_key);
        }
        Ok(token)
    }

    pub fn live_lock_count(&self) -> usize {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.drop_idle_locks();
        state.locks.len()
    }

    fn key_lock(&self, cache_key: &str) -> KeyLock {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.drop_idle_locks();
        Arc::clone(state.locks.entry(cache_key.to_owned()).or_default())
    }

    fn store_token(
        &self,
        cache_key: &str,
        token: &SecretString,
        ttl: u32,
    ) -> Result<(), DatabricksOauthError> {
        let credential = Credential {
            token: token.clone(),
            headers: BTreeMap::new(),
        };
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state
            .tokens
            .set(
                cache_key,
                CacheValue::Credential(CredentialHandle(Arc::new(credential))),
                Some(f64::from(ttl)),
            )
            .map_err(DatabricksOauthError::Cache)?;
        state.drop_idle_locks();
        drop(state);
        Ok(())
    }

    fn cached_token(&self, cache_key: &str) -> Result<Option<SecretString>, DatabricksOauthError> {
        let cached = self
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .tokens
            .get(cache_key)
            .map_err(DatabricksOauthError::Cache)?;
        Ok(match cached {
            Some(CacheValue::Credential(handle)) => Some(handle.0.token.clone()),
            Some(
                CacheValue::Bytes(_)
                | CacheValue::Null
                | CacheValue::Boolean(_)
                | CacheValue::Integer(_)
                | CacheValue::Float(_)
                | CacheValue::Text(_)
                | CacheValue::Array(_)
                | CacheValue::Object(_),
            )
            | None => None,
        })
    }
}
