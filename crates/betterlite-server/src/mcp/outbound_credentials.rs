pub mod authz_code_refresher;
pub mod cache_codec;
pub mod cached_store;
pub mod client_credentials;
pub mod distributed_lock;
pub mod dual_cache;
pub mod per_user_oauth_store;
pub mod presented_store;
pub mod refresh_coordinator;
pub mod refresh_coordinator_factory;
pub mod refresh_store;
pub mod sso_assertion_refresher;
mod tests;
pub mod token_endpoint;
pub mod token_exchange_provider;
pub mod token_exchanger;
pub mod v2_store;

use std::fmt;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, NaiveDate, NaiveDateTime, TimeDelta, Utc};
use futures_util::future::BoxFuture;
use indexmap::IndexMap;
use thiserror::Error;

pub const DEFAULT_MAX_SIZE: usize = 4096;
pub const DEFAULT_EXPIRY_SKEW: Duration = Duration::from_secs(60);
// A cached value without this prefix is a legacy bearer-only entry from v1 and stays readable through the cutover.
pub const BOUND_PREFIX: &str = "litellm-bound-oauth-v1:";
pub const PER_USER_TOKEN_KEY_PREFIX: &str = "mcp:per_user_token:";

#[derive(Clone, Eq, PartialEq)]
pub struct OAuthToken {
    pub access_token: String,
    pub expires_at: Option<DateTime<Utc>>,
    pub refresh_token: Option<String>,
    pub scopes: Vec<String>,
    pub identity_binding_proof: Option<String>,
}

impl fmt::Debug for OAuthToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OAuthToken")
            .field("access_token", &"***")
            .field("expires_at", &self.expires_at)
            .field("has_refresh_token", &self.refresh_token.is_some())
            .field("scopes", &self.scopes)
            .finish_non_exhaustive()
    }
}

impl OAuthToken {
    #[must_use]
    pub fn is_expired(&self, now: DateTime<Utc>, expiry_skew: Duration) -> bool {
        let Some(expires_at) = self.expires_at else {
            return false;
        };
        let Ok(skew) = TimeDelta::from_std(expiry_skew) else {
            return true;
        };
        expires_at
            .checked_sub_signed(skew)
            .is_none_or(|deadline| now >= deadline)
    }
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
#[error("{detail}")]
pub struct TokenStoreUnavailable {
    pub detail: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct TokenKey {
    pub user_id: String,
    pub server_id: String,
}

#[async_trait]
pub trait OAuthTokenStore: Send + Sync {
    async fn fetch(
        &self,
        user_id: &str,
        server_id: &str,
    ) -> Result<Option<OAuthToken>, TokenStoreUnavailable>;
}

#[async_trait]
pub trait InvalidatableOAuthTokenStore: OAuthTokenStore {
    async fn invalidate(&self, user_id: &str, server_id: &str);
}

#[async_trait]
pub trait TokenRefresher: Send + Sync {
    async fn refresh(
        &self,
        user_id: &str,
        server_id: &str,
        token: &OAuthToken,
    ) -> Result<Option<OAuthToken>, TokenStoreUnavailable>;
}

#[async_trait]
pub trait TokenCacheBackend: Send + Sync {
    async fn get(&self, key: &TokenKey) -> Option<OAuthToken>;
    async fn set(&self, key: &TokenKey, token: &OAuthToken, ttl: Duration);
    async fn delete(&self, key: &TokenKey);
}

pub type RefreshFuture<'call, Output, Error> = BoxFuture<'call, Result<Output, Error>>;
pub type RefreshStep<'call, Output, Error> =
    Box<dyn FnOnce() -> RefreshFuture<'call, Output, Error> + Send + 'call>;

// Callers for one key share a single-flight refresh, so the provider sees one refresh per key.
#[async_trait]
pub trait RefreshCoordinator: Send + Sync {
    type Output: Clone + Send + 'static;
    type Error: Clone + Send + 'static;

    async fn run(
        &self,
        key: TokenKey,
        refresh: RefreshStep<'_, Self::Output, Self::Error>,
        reread: RefreshStep<'_, Self::Output, Self::Error>,
    ) -> Result<Self::Output, Self::Error>;
}

pub trait WallClock: Send + Sync {
    fn now(&self) -> DateTime<Utc>;
}

#[derive(Debug, Default)]
pub struct SystemWallClock;

impl WallClock for SystemWallClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

pub struct InMemoryTokenCacheBackend {
    pub max_size: usize,
    pub clock: Arc<dyn WallClock>,
    pub entries: Mutex<IndexMap<TokenKey, (OAuthToken, DateTime<Utc>)>>,
}

// A timezone-naive ISO expiry is stored as UTC, so anchor it before it names an instant.
#[must_use]
pub fn iso_to_epoch(text: &str) -> Option<DateTime<Utc>> {
    if let Ok(parsed) = DateTime::parse_from_rfc3339(text) {
        return Some(parsed.with_timezone(&Utc));
    }
    for format in ["%Y-%m-%dT%H:%M:%S%.f", "%Y-%m-%d %H:%M:%S%.f"] {
        if let Ok(naive) = NaiveDateTime::parse_from_str(text, format) {
            return Some(naive.and_utc());
        }
    }
    NaiveDate::parse_from_str(text, "%Y-%m-%d")
        .ok()
        .and_then(|date| date.and_hms_opt(0, 0, 0))
        .map(|naive| naive.and_utc())
}

#[async_trait]
impl TokenCacheBackend for InMemoryTokenCacheBackend {
    async fn get(&self, key: &TokenKey) -> Option<OAuthToken> {
        let now = self.clock.now();
        let mut entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some((token, valid_until)) = entries.get(key)
            && now < *valid_until
        {
            return Some(token.clone());
        }
        drop(entries.shift_remove(key));
        None
    }

    async fn set(&self, key: &TokenKey, token: &OAuthToken, ttl: Duration) {
        let mut entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        if !entries.contains_key(key) && entries.len() >= self.max_size {
            // Evict the oldest insertion instead of clearing the cache, so only one key re-reads.
            drop(entries.shift_remove_index(0));
        }
        let valid_until = TimeDelta::from_std(ttl)
            .ok()
            .and_then(|lifetime| self.clock.now().checked_add_signed(lifetime))
            .unwrap_or(DateTime::<Utc>::MAX_UTC);
        entries.insert(key.clone(), (token.clone(), valid_until));
    }

    async fn delete(&self, key: &TokenKey) {
        let mut entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        drop(entries.shift_remove(key));
    }
}
