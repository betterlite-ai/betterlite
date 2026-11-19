pub mod auth;
pub mod error;
pub mod per_user;
mod request;
mod tests;

use std::sync::{Arc, Mutex, PoisonError};

use betterlite_mcp::oauth_resource::resource_identity::{
    UpstreamServer, resolve_upstream_resource,
};
use betterlite_mcp::server_record::MCPServer;
use chrono::{DateTime, TimeDelta, Utc};
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};
use sha2::{Digest as _, Sha256};
use tokio::sync::Mutex as AsyncMutex;

use crate::mcp::outbound_credentials::WallClock;
use crate::mcp::outbound_credentials::cache_codec::OAuthTokenCacheCodec;
use crate::mcp::outbound_credentials::client_credentials::token_post::{
    M2mTokenEndpointPost, TokenEndpointOutcome,
};
use auth::{McpAuthOverride, McpAuthValue};
use error::OAuthCacheError;
use per_user::PerUserTokenBackend;
use request::{auth_method_name, expires_in_seconds, token_request_form, token_request_inputs};

pub const OAUTH2_TOKEN_CACHE_MAX_SIZE: usize = 200;
pub const OAUTH2_TOKEN_CACHE_MAX_LOCKS: usize = 1024;
pub const OAUTH2_TOKEN_CACHE_DEFAULT_TTL: i64 = 3600;
pub const OAUTH2_TOKEN_CACHE_MIN_TTL: i64 = 10;
pub const OAUTH2_TOKEN_TTL_BUFFER: i64 = 60;
pub const PER_USER_TOKEN_DEFAULT_TTL: i64 = 43_200;
pub const PER_USER_TTL_BUFFER: i64 = 60;
// The identity material is \x00-joined exactly as LiteLLM joins it, so both sides hash the same bytes.
const TOKEN_IDENTITY_SEPARATOR: &str = "\u{0}";

pub struct MCPOAuth2TokenCache {
    poster: Arc<dyn M2mTokenEndpointPost>,
    clock: Arc<dyn WallClock>,
    entries: Mutex<IndexMap<String, (String, DateTime<Utc>)>>,
    locks: Mutex<IndexMap<String, Arc<AsyncMutex<()>>>>,
}

impl MCPOAuth2TokenCache {
    #[must_use]
    pub fn new(poster: Arc<dyn M2mTokenEndpointPost>, clock: Arc<dyn WallClock>) -> Self {
        Self {
            poster,
            clock,
            entries: Mutex::new(IndexMap::new()),
            locks: Mutex::new(IndexMap::new()),
        }
    }

    // The sha256 covers every input the token request puts on the wire, so a config change misses the cache.
    #[must_use]
    pub fn token_identity(server: &MCPServer) -> String {
        let scopes = server.fields.scopes.as_deref().unwrap_or(&[]).join(" ");
        let upstream = UpstreamServer {
            upstream_resource: server.fields.upstream_resource.as_deref(),
            url: server.fields.url.as_deref(),
            server_id: server.fields.server_id.as_str(),
        };
        let material = [
            server.effective_token_url().unwrap_or_default().to_owned(),
            server.fields.client_id.clone().unwrap_or_default(),
            client_secret(server).unwrap_or_default().to_owned(),
            scopes,
            resolve_upstream_resource(&upstream).unwrap_or_default(),
            server
                .fields
                .token_endpoint_auth_method
                .map(auth_method_name)
                .unwrap_or_default()
                .to_owned(),
        ]
        .join(TOKEN_IDENTITY_SEPARATOR);
        format!(
            "{}:{}",
            server.fields.server_id,
            hex::encode(Sha256::digest(material.as_bytes()))
        )
    }

    #[must_use]
    pub fn has_client_credentials_config(server: &MCPServer) -> bool {
        server
            .fields
            .client_id
            .as_deref()
            .is_some_and(|value| !value.is_empty())
            && client_secret(server).is_some_and(|value| !value.is_empty())
            && server
                .effective_token_url()
                .is_some_and(|url| !url.is_empty())
    }

    pub async fn async_get_token(
        &self,
        server: &MCPServer,
    ) -> Result<Option<String>, OAuthCacheError> {
        if !server.has_client_credentials() || !Self::has_client_credentials_config(server) {
            return Ok(None);
        }
        let identity = Self::token_identity(server);
        if let Some(token) = self.cached(&identity) {
            return Ok(Some(token));
        }
        let lock = self.lock_for(&identity);
        let _guard = lock.lock().await;
        if let Some(token) = self.cached(&identity) {
            return Ok(Some(token));
        }
        let (token, ttl) = self.fetch_token(server).await?;
        self.insert(identity, token.clone(), ttl);
        Ok(Some(token))
    }

    // POSTs grant_type=client_credentials to the effective token URL and returns the token with its cache TTL.
    pub async fn fetch_token(&self, server: &MCPServer) -> Result<(String, i64), OAuthCacheError> {
        let server_id = server.fields.server_id.as_str();
        let (token_url, client_id, secret) = token_request_inputs(server)?;
        let request = token_request_form(server, client_id, &secret).ok_or_else(|| {
            OAuthCacheError::RequestBuild {
                server_id: server_id.to_owned(),
            }
        })?;
        let body = match self
            .poster
            .post(token_url, &request.form, &request.headers)
            .await
        {
            TokenEndpointOutcome::Success { body } => body,
            TokenEndpointOutcome::Denied { status_code, .. } => {
                return Err(OAuthCacheError::Denied {
                    server_id: server_id.to_owned(),
                    status_code,
                });
            }
            TokenEndpointOutcome::Unreachable { detail } => {
                return Err(OAuthCacheError::Unreachable {
                    server_id: server_id.to_owned(),
                    detail,
                });
            }
        };
        let access_token = body
            .access_token_text()
            .filter(|text| !text.is_empty())
            .ok_or_else(|| OAuthCacheError::MissingAccessToken {
                server_id: server_id.to_owned(),
            })?;
        let ttl = expires_in_seconds(body.expires_in.as_ref())
            .saturating_sub(OAUTH2_TOKEN_TTL_BUFFER)
            .max(OAUTH2_TOKEN_CACHE_MIN_TTL);
        Ok((access_token.to_owned(), ttl))
    }

    pub fn invalidate(&self, server_id: &str) {
        let prefix = format!("{server_id}:");
        let mut entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        entries.retain(|key, _value| !key.starts_with(&prefix));
    }

    fn cached(&self, identity: &str) -> Option<String> {
        let now = self.clock.now();
        let mut entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some((token, valid_until)) = entries.get(identity)
            && now < *valid_until
        {
            return Some(token.clone());
        }
        drop(entries.shift_remove(identity));
        None
    }

    fn insert(&self, identity: String, token: String, ttl: i64) {
        let valid_until = TimeDelta::try_seconds(ttl)
            .and_then(|lifetime| self.clock.now().checked_add_signed(lifetime))
            .unwrap_or(DateTime::<Utc>::MAX_UTC);
        let mut entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        if !entries.contains_key(&identity) && entries.len() >= OAUTH2_TOKEN_CACHE_MAX_SIZE {
            drop(entries.shift_remove_index(0));
        }
        drop(entries.insert(identity, (token, valid_until)));
    }

    // The lock map is capped, so a stream of distinct configs cannot grow it without bound.
    fn lock_for(&self, identity: &str) -> Arc<AsyncMutex<()>> {
        let mut locks = self.locks.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(existing) = locks.get(identity) {
            return Arc::clone(existing);
        }
        if locks.len() >= OAUTH2_TOKEN_CACHE_MAX_LOCKS {
            drop(locks.shift_remove_index(0));
        }
        let lock = Arc::new(AsyncMutex::new(()));
        drop(locks.insert(identity.to_owned(), Arc::clone(&lock)));
        lock
    }
}

pub struct MCPPerUserTokenCache<B: PerUserTokenBackend> {
    pub backend: B,
    pub codec: OAuthTokenCacheCodec,
}

#[must_use]
pub fn compute_per_user_token_ttl(server: &MCPServer, expires_in: Option<i64>) -> i64 {
    let lifetime = expires_in.map(|value| value.saturating_sub(PER_USER_TTL_BUFFER));
    match (server.fields.token_storage_ttl_seconds, lifetime) {
        (Some(configured), Some(bound)) => configured.min(bound).max(1),
        (Some(configured), None) => configured.max(1),
        (None, Some(bound)) => bound.max(1),
        (None, None) => PER_USER_TOKEN_DEFAULT_TTL,
    }
}

// Priority: the caller's mcp_auth_header, then an OAuth2 client_credentials token, then authentication_token.
pub async fn resolve_mcp_auth(
    server: &MCPServer,
    mcp_auth_header: Option<&McpAuthOverride>,
    cache: &MCPOAuth2TokenCache,
) -> Result<Option<McpAuthValue>, OAuthCacheError> {
    if let Some(header) = mcp_auth_header.filter(|value| !value.is_empty()) {
        return Ok(Some(match header {
            McpAuthOverride::Text(value) => McpAuthValue::Text(value.clone()),
            McpAuthOverride::Headers(headers) => McpAuthValue::Headers(headers.clone()),
        }));
    }
    if server.has_client_credentials() {
        let token = cache.async_get_token(server).await?;
        return Ok(token.map(|value| McpAuthValue::Text(SecretString::from(value))));
    }
    Ok(server
        .fields
        .authentication_token
        .as_ref()
        .map(|credential| McpAuthValue::Text(credential.0.0.clone())))
}

#[must_use]
pub fn resolved_token_header<'server>(
    server: &'server MCPServer,
    mcp_auth_header: Option<&McpAuthOverride>,
) -> Option<&'server str> {
    match mcp_auth_header {
        Some(header) if !header.is_empty() => None,
        Some(_) | None => server.fields.upstream_token_header.as_deref(),
    }
}

fn client_secret(server: &MCPServer) -> Option<&str> {
    server
        .fields
        .client_secret
        .as_ref()
        .map(|credential| credential.0.0.expose_secret())
}
