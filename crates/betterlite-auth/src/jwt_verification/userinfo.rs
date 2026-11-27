use std::str;

use async_trait::async_trait;
use betterlite_http::error::HttpError;
use betterlite_http::proxy::ProxiedClient;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use sha2::{Digest as _, Sha256};
use thiserror::Error;

use super::settings::JwtVerificationSettings;
use crate::auth_cache::AuthCacheError;
use crate::jwt_keys::MAX_DOCUMENT_BYTES;
use crate::jwt_keys::cache::KeyCache;
use crate::jwt_keys::http::FetchError;
use crate::jwt_keys::userinfo_document;

pub const USERINFO_CACHE_KEY_PREFIX: &str = "oidc_userinfo_";
pub const USERINFO_ACCEPT: &str = "application/json";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UserinfoReply {
    pub status: u16,
    pub body: Vec<u8>,
}

// One GET with the bearer token and JSON accept; there is no retry and no stale copy on this path.
#[async_trait]
pub trait UserinfoHttp: Send + Sync {
    async fn get(
        &self,
        url: &str,
        authorization: &str,
        accept: &str,
    ) -> Result<UserinfoReply, FetchError>;
    fn close(&self);
}

#[derive(Debug, Error)]
pub enum UserinfoError {
    #[error(
        "OIDC UserInfo endpoint not configured. Set 'oidc_userinfo_endpoint' in JWT auth config."
    )]
    NotConfigured,
    #[error(transparent)]
    Cache(#[from] AuthCacheError),
    #[error("Failed to fetch OIDC UserInfo: {detail}")]
    Failed { detail: String },
}

// oidc_userinfo_<sha256 token hex>, so the bearer credential itself never becomes a cache key.
#[must_use]
pub fn cache_key(token: &str) -> String {
    let digest = Sha256::digest(token.as_bytes());
    format!("{USERINFO_CACHE_KEY_PREFIX}{}", hex::encode(digest))
}

// JWTHandler.get_oidc_userinfo: endpoint check, cache read, one fetch, success log, cache write, then return.
pub async fn fetch_userinfo(
    settings: &JwtVerificationSettings,
    cache: &dyn KeyCache,
    http: &dyn UserinfoHttp,
    token: &str,
) -> Result<IndexMap<String, Json>, UserinfoError> {
    let Some(endpoint) = settings.userinfo_endpoint() else {
        return Err(UserinfoError::NotConfigured);
    };
    if endpoint.is_empty() {
        return Err(UserinfoError::NotConfigured);
    }
    let key = cache_key(token);
    // a cache read failure is the cache's own error, never wrapped as a fetch failure
    if let Some(cached) = cache.read_value(&key).await? {
        match cached {
            Json::Object(fields) => {
                tracing::debug!(
                    target: "LiteLLM Proxy",
                    "Returning cached OIDC UserInfo"
                );
                return Ok(fields);
            }
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {}
        }
    }
    tracing::debug!(
        target: "LiteLLM Proxy",
        "Calling OIDC UserInfo endpoint: {endpoint}"
    );
    let fetched = async {
        let authorization = format!("Bearer {token}");
        let reply = http
            .get(endpoint, &authorization, USERINFO_ACCEPT)
            .await
            .map_err(|error| error.to_string())?;
        if reply.status != 200 {
            let text = str::from_utf8(&reply.body).unwrap_or_default();
            return Err(format!(
                "OIDC UserInfo endpoint returned status {}: {text}",
                reply.status
            ));
        }
        let userinfo = userinfo_document(&reply.body).map_err(|error| error.to_string())?;
        // a privacy deviation from LiteLLM: the outcome is logged, the identity document never is
        tracing::debug!(target: "LiteLLM Proxy", "Received OIDC UserInfo");
        let stored = Json::Object(userinfo.clone());
        cache
            .write_value(&key, &stored, settings.userinfo_ttl())
            .await
            .map_err(|error| error.to_string())?;
        Ok(userinfo)
    }
    .await;
    match fetched {
        Ok(userinfo) => Ok(userinfo),
        Err(detail) => {
            tracing::error!(
                target: "LiteLLM Proxy",
                "Error fetching OIDC UserInfo: {detail}"
            );
            Err(UserinfoError::Failed { detail })
        }
    }
}

pub struct ProxiedUserinfoHttp {
    pub client: ProxiedClient,
}

#[async_trait]
impl UserinfoHttp for ProxiedUserinfoHttp {
    async fn get(
        &self,
        url: &str,
        authorization: &str,
        accept: &str,
    ) -> Result<UserinfoReply, FetchError> {
        let request = self
            .client
            .client
            .get(url)
            .header(http::header::AUTHORIZATION, authorization)
            .header(http::header::ACCEPT, accept)
            .build()
            .map_err(|source| FetchError::Transport(HttpError::Request(source)))?;
        let body_limit = self.client.limits.body_bytes.min(MAX_DOCUMENT_BYTES);
        match self.client.execute_buffered(request, body_limit).await {
            Ok(response) => Ok(UserinfoReply {
                status: response.status,
                body: response.body,
            }),
            Err(error) => Err(FetchError::Failed(error)),
        }
    }

    fn close(&self) {
        // the outbound client belongs to the server's shared state and outlives this verifier
    }
}
