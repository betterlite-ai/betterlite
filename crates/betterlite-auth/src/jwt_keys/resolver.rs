use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::Mutex as AsyncMutex;

use betterlite_models::decisions::Json;

use super::cache::KeyCache;
use super::clock::Clock;
use super::errors::KeyError;
use super::http::JwksHttp;
use super::settings::JwtKeySettings;
use refresh::refresh_or_serve_stale;

pub mod fetch;
pub mod keys;
pub mod refresh;

pub const STALE_CACHE_KEY_PREFIX: &str = "litellm_stale_";
pub const STALE_WRITTEN_AT_PREFIX: &str = "litellm_stale_written_at_";
pub const UNREACHABLE_CACHE_KEY_PREFIX: &str = "litellm_jwks_unreachable_";
pub const JWKS_FETCH_ATTEMPTS: u32 = 3;
pub const JWKS_FETCH_RETRY_BACKOFF: Duration = Duration::from_millis(250);
pub const UNREACHABLE_BACKOFF_SECONDS: f64 = 30.0;
pub const DISCOVERY_CACHE_KEY_PREFIX: &str = "litellm_oidc_discovery_";
pub const KEYS_CACHE_KEY_PREFIX: &str = "litellm_jwt_auth_keys_";
pub const DISCOVERY_DOCUMENT_MARKER: &str = ".well-known/openid-configuration";

// The refresh a cache miss runs; an enum rather than a closure so nothing is boxed per request.
#[derive(Clone, Copy)]
pub enum Refresh<'url> {
    Discovery(&'url str),
    Keys(&'url str),
}

pub struct JwksResolver {
    settings: Arc<JwtKeySettings>,
    cache: Arc<dyn KeyCache>,
    http: Arc<dyn JwksHttp>,
    clock: Arc<dyn Clock>,
    refresh_locks: Mutex<BTreeMap<String, Arc<AsyncMutex<()>>>>,
}

impl JwksResolver {
    #[must_use]
    pub fn new(
        settings: Arc<JwtKeySettings>,
        cache: Arc<dyn KeyCache>,
        http: Arc<dyn JwksHttp>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            settings,
            cache,
            http,
            clock,
            refresh_locks: Mutex::new(BTreeMap::new()),
        }
    }

    // get_public_key: absent raises, empty is an empty list, and the trimmed URLs are tried in order.
    pub async fn get_public_key(&self, kid: Option<&str>) -> Result<Json, KeyError> {
        let Some(raw) = self.settings.public_key_url.as_deref() else {
            return Err(KeyError::MissingUrl);
        };
        let urls = keys::plain_urls(raw);
        for url in &urls {
            match self.keys_for_url(url, kid).await {
                Ok(key) => return Ok(key),
                Err(error @ KeyError::NoMatchingKey { .. }) => {
                    tracing::debug!(
                        target: "LiteLLM Proxy",
                        "JWT Auth: No matching public key found at {url}: {error}"
                    );
                }
                Err(error @ KeyError::Unreachable { .. }) => {
                    tracing::error!(
                        target: "LiteLLM Proxy",
                        "JWT Auth: JWKS endpoint {url} unreachable: {error}"
                    );
                    return Err(error);
                }
                Err(other) => return Err(other),
            }
        }
        let rendered = urls
            .iter()
            .map(|url| format!("'{url}'"))
            .collect::<Vec<_>>()
            .join(", ");
        Err(KeyError::NoMatchingKey {
            keys: format!("[{rendered}]"),
            kid: kid.map(str::to_owned),
        })
    }

    // _get_public_key_from_jwks_url: resolve discovery, then read the keys cache and pick the kid.
    pub async fn keys_for_url(&self, jwks_url: &str, kid: Option<&str>) -> Result<Json, KeyError> {
        let resolved = self.resolve_jwks_url(jwks_url).await?;
        let cache_key = format!("{KEYS_CACHE_KEY_PREFIX}{resolved}");
        let ttl = self.settings.public_key_ttl;
        let log_context = format!("kid={}", kid.unwrap_or("None"));
        let keys = self
            .cached_with_stale_fallback(&cache_key, ttl, Refresh::Keys(&resolved), &log_context)
            .await?;
        keys::parse_keys(&keys, kid).ok_or_else(|| KeyError::NoMatchingKey {
            keys: resolved,
            kid: kid.map(str::to_owned),
        })
    }

    // _resolve_jwks_url: a substring test, not a suffix one, so a query string still counts.
    pub async fn resolve_jwks_url(&self, url: &str) -> Result<String, KeyError> {
        if !url.contains(DISCOVERY_DOCUMENT_MARKER) {
            return Ok(url.to_owned());
        }
        let cache_key = format!("{DISCOVERY_CACHE_KEY_PREFIX}{url}");
        let ttl = self.settings.public_key_ttl;
        let value = self
            .cached_with_stale_fallback(
                &cache_key,
                ttl,
                Refresh::Discovery(url),
                "an OIDC discovery lookup",
            )
            .await?;
        match value {
            Json::Text(jwks_uri) => Ok(jwks_uri),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
                Err(KeyError::DiscoveryMissingUri {
                    url: url.to_owned(),
                })
            }
        }
    }

    // _cached_with_stale_fallback: a nonnull value wins, and a miss refreshes once per key under its lock.
    async fn cached_with_stale_fallback(
        &self,
        cache_key: &str,
        ttl: f64,
        refresh: Refresh<'_>,
        log_context: &str,
    ) -> Result<Json, KeyError> {
        // a stored null reads back as None in Python, so it is a miss here too
        if let Some(cached) = self
            .cache
            .read_value(cache_key)
            .await?
            .filter(|value| !matches!(value, Json::Null))
        {
            return Ok(cached);
        }
        let lock = self.refresh_lock(cache_key);
        let _guard = lock.lock().await;
        if let Some(cached) = self
            .cache
            .read_value(cache_key)
            .await?
            .filter(|value| !matches!(value, Json::Null))
        {
            return Ok(cached);
        }
        refresh_or_serve_stale(self, cache_key, ttl, refresh, log_context).await
    }

    fn refresh_lock(&self, cache_key: &str) -> Arc<AsyncMutex<()>> {
        let mut locks = match self.refresh_locks.lock() {
            Ok(locks) => locks,
            Err(poisoned) => poisoned.into_inner(),
        };
        Arc::clone(locks.entry(cache_key.to_owned()).or_default())
    }
}
