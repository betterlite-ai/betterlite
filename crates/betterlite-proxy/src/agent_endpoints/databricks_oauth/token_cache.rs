use std::collections::BTreeMap;
use std::sync::Arc;

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

pub struct DatabricksAppOAuthTokenCache {
    client: ProxiedClient,
    clock: Clock,
    tokens: Mutex<LocalTtlCache>,
    // one lock per live cache key, so concurrent callers share a single token request
    locks: Mutex<BTreeMap<String, KeyLock>>,
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

// The lock of a key that left the cache goes with it, unless a caller still waits on it.
fn drop_idle_locks(tokens: &mut LocalTtlCache, locks: &mut BTreeMap<String, KeyLock>) {
    locks.retain(|key, lock| {
        Arc::strong_count(lock) > 1 || tokens.get(key).is_ok_and(|found| found.is_some())
    });
}

impl DatabricksAppOAuthTokenCache {
    pub fn new(client: ProxiedClient, clock: Clock) -> Result<Self, DatabricksOauthError> {
        let tokens = new_token_cache(&clock).map_err(DatabricksOauthError::Cache)?;
        Ok(Self {
            client,
            clock,
            tokens: Mutex::new(tokens),
            locks: Mutex::new(BTreeMap::new()),
        })
    }

    pub async fn flush_cache(&self) -> Result<(), DatabricksOauthError> {
        let fresh = new_token_cache(&self.clock).map_err(DatabricksOauthError::Cache)?;
        *self.tokens.lock().await = fresh;
        self.locks.lock().await.clear();
        Ok(())
    }

    pub async fn async_get_token(
        &self,
        config: &DatabricksAppOAuthConfig,
    ) -> Result<SecretString, DatabricksOauthError> {
        let cache_key = config.cache_key();
        if let Some(token) = self.cached_token(&cache_key).await? {
            return Ok(token);
        }
        let lock = self.key_lock(&cache_key).await;
        let _coalescing = lock.lock().await;
        if let Some(token) = self.cached_token(&cache_key).await? {
            return Ok(token);
        }
        let (token, ttl) = fetch_token(&self.client, config).await?;
        if ttl > 0 {
            self.store_token(&cache_key, &token, ttl).await?;
        } else {
            self.locks.lock().await.remove(&cache_key);
        }
        Ok(token)
    }

    // The number of keys that still hold a token or have a caller waiting.
    pub async fn live_lock_count(&self) -> usize {
        let mut tokens = self.tokens.lock().await;
        let mut locks = self.locks.lock().await;
        drop_idle_locks(&mut tokens, &mut locks);
        locks.len()
    }

    async fn key_lock(&self, cache_key: &str) -> KeyLock {
        let mut tokens = self.tokens.lock().await;
        let mut locks = self.locks.lock().await;
        drop_idle_locks(&mut tokens, &mut locks);
        Arc::clone(locks.entry(cache_key.to_owned()).or_default())
    }

    async fn store_token(
        &self,
        cache_key: &str,
        token: &SecretString,
        ttl: u32,
    ) -> Result<(), DatabricksOauthError> {
        let credential = Credential {
            token: token.clone(),
            headers: BTreeMap::new(),
        };
        let mut tokens = self.tokens.lock().await;
        tokens
            .set(
                cache_key,
                CacheValue::Credential(CredentialHandle(Arc::new(credential))),
                Some(f64::from(ttl)),
            )
            .map_err(DatabricksOauthError::Cache)?;
        let mut locks = self.locks.lock().await;
        drop_idle_locks(&mut tokens, &mut locks);
        drop(tokens);
        Ok(())
    }

    async fn cached_token(
        &self,
        cache_key: &str,
    ) -> Result<Option<SecretString>, DatabricksOauthError> {
        let cached = self
            .tokens
            .lock()
            .await
            .get(cache_key)
            .map_err(DatabricksOauthError::Cache)?;
        Ok(match cached {
            Some(CacheValue::Credential(handle)) => Some(handle.0.token.clone()),
            _ => None,
        })
    }
}
