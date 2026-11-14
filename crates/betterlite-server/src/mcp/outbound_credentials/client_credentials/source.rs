mod tests;

use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_mcp::credential_contracts::auth_config::oauth::ClientCredentialsConfig;
use betterlite_mcp::credential_contracts::errors::{CredError, CredResult};
use chrono::{DateTime, TimeDelta, Utc};
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};
use subtle::ConstantTimeEq as _;
use tokio::sync::Mutex as AsyncMutex;

use super::grant::{PreparedGrant, parse_expires_in, parse_granted_scopes, prepare_grant};
use super::token_post::{M2mTokenEndpointPost, TokenEndpointOutcome, TokenResponseBody};
use crate::mcp::outbound_credentials::{
    DEFAULT_MAX_SIZE, InMemoryTokenCacheBackend, OAuthToken, TokenCacheBackend, TokenKey, WallClock,
};

#[derive(Debug, Clone, Copy)]
pub struct Tunables {
    pub default_ttl_seconds: u64,
    pub expiry_skew_seconds: u64,
    pub min_cache_seconds: u64,
    pub max_locks: usize,
}

impl Default for Tunables {
    fn default() -> Self {
        Self {
            default_ttl_seconds: 300,
            expiry_skew_seconds: 60,
            min_cache_seconds: 10,
            max_locks: 1024,
        }
    }
}

pub struct ClientCredentialsTokenSource {
    pub poster: Arc<dyn M2mTokenEndpointPost>,
    pub backend: Arc<dyn TokenCacheBackend>,
    pub clock: Arc<dyn WallClock>,
    pub tunables: Tunables,
    pub locks: Mutex<IndexMap<String, Arc<AsyncMutex<()>>>>,
}

impl ClientCredentialsTokenSource {
    pub fn new(
        poster: Arc<dyn M2mTokenEndpointPost>,
        backend: Option<Arc<dyn TokenCacheBackend>>,
        clock: Arc<dyn WallClock>,
        tunables: Tunables,
    ) -> Self {
        let cache = backend.unwrap_or_else(|| {
            Arc::new(InMemoryTokenCacheBackend {
                max_size: DEFAULT_MAX_SIZE,
                clock: Arc::clone(&clock),
                entries: Mutex::default(),
            })
        });
        Self {
            poster,
            backend: cache,
            clock,
            tunables,
            locks: Mutex::new(IndexMap::new()),
        }
    }

    pub async fn get(
        &self,
        server_id: &str,
        config: &ClientCredentialsConfig,
    ) -> CredResult<OAuthToken> {
        let grant = prepare_grant(config)?;
        let key = TokenKey {
            user_id: grant.identity_key.clone(),
            server_id: server_id.to_owned(),
        };
        if let Some(token) = self.backend.get(&key).await {
            return Ok(token);
        }
        let lock = self.lock_for(server_id);
        let _held = lock.lock().await;
        if let Some(token) = self.backend.get(&key).await {
            return Ok(token);
        }
        self.fetch_and_cache(&key, &grant).await
    }

    pub async fn refetch(
        &self,
        server_id: &str,
        config: &ClientCredentialsConfig,
        failed_access_token: &SecretString,
    ) -> Option<SecretString> {
        let grant = prepare_grant(config).ok()?;
        let key = TokenKey {
            user_id: grant.identity_key.clone(),
            server_id: server_id.to_owned(),
        };
        let lock = self.lock_for(server_id);
        let _held = lock.lock().await;
        if let Some(cached) = self.backend.get(&key).await
            && !bool::from(
                cached
                    .access_token
                    .as_bytes()
                    .ct_eq(failed_access_token.expose_secret().as_bytes()),
            )
        {
            return Some(SecretString::from(cached.access_token));
        }
        self.backend.delete(&key).await;
        match self.fetch_and_cache(&key, &grant).await {
            Ok(token) => Some(SecretString::from(token.access_token)),
            Err(_failed) => None,
        }
    }

    fn lock_for(&self, server_id: &str) -> Arc<AsyncMutex<()>> {
        let mut locks = self.locks.lock().unwrap_or_else(PoisonError::into_inner);
        if !locks.contains_key(server_id) && locks.len() >= self.tunables.max_locks {
            // A dropped lock that a task still holds only lets one extra grant run, so eviction stays safe.
            drop(locks.shift_remove_index(0));
        }
        Arc::clone(
            locks
                .entry(server_id.to_owned())
                .or_insert_with(|| Arc::new(AsyncMutex::new(()))),
        )
    }

    async fn fetch_and_cache(
        &self,
        key: &TokenKey,
        grant: &PreparedGrant,
    ) -> CredResult<OAuthToken> {
        let outcome = self
            .poster
            .post(&grant.token_url, &grant.form, &grant.headers)
            .await;
        let body = match outcome {
            TokenEndpointOutcome::Unreachable { detail } => {
                return Err(CredError::UpstreamUnavailable(format!(
                    "OAuth2 token endpoint unreachable: {detail}"
                )));
            }
            TokenEndpointOutcome::Denied {
                status_code,
                detail,
            } => {
                return if status_code >= 500 {
                    Err(CredError::UpstreamUnavailable(format!(
                        "OAuth2 token endpoint failed: {detail}"
                    )))
                } else {
                    Err(CredError::Misconfigured(format!(
                        "OAuth2 client_credentials grant rejected: {detail}"
                    )))
                };
            }
            TokenEndpointOutcome::Success { body } => body,
        };
        self.cache_token(key, &body).await
    }

    async fn cache_token(
        &self,
        key: &TokenKey,
        body: &TokenResponseBody,
    ) -> CredResult<OAuthToken> {
        let access_token = match body.access_token_text() {
            Some(text) if !text.is_empty() => text,
            Some(_) | None => {
                return Err(CredError::Misconfigured(
                    "OAuth2 token response is missing 'access_token'".to_owned(),
                ));
            }
        };
        let expires_in = parse_expires_in(body.expires_in.as_ref());
        let token = OAuthToken {
            access_token: access_token.to_owned(),
            expires_at: expires_in.map(|seconds| self.expiry(seconds)),
            refresh_token: None,
            scopes: parse_granted_scopes(body.scope.as_ref()),
            identity_binding_proof: None,
        };
        if let Some(ttl) = self.cache_ttl(expires_in) {
            self.backend.set(key, &token, ttl).await;
        }
        Ok(token)
    }

    fn expiry(&self, expires_in: i64) -> DateTime<Utc> {
        let now = self.clock.now();
        TimeDelta::try_seconds(expires_in)
            .and_then(|lifetime| now.checked_add_signed(lifetime))
            .unwrap_or(DateTime::<Utc>::MAX_UTC)
    }

    fn cache_ttl(&self, expires_in: Option<i64>) -> Option<Duration> {
        let Some(seconds) = expires_in else {
            return Some(Duration::from_secs(self.tunables.default_ttl_seconds));
        };
        let lifetime = Duration::from_secs(u64::try_from(seconds).ok()?);
        let skew = Duration::from_secs(self.tunables.expiry_skew_seconds);
        let floor = Duration::from_secs(self.tunables.min_cache_seconds);
        let ttl = lifetime.saturating_sub(skew).max(lifetime.min(floor));
        (ttl > Duration::ZERO).then_some(ttl)
    }
}
