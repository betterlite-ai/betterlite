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
use crate::jwt_keys::cache::KeyCache;
use crate::jwt_keys::http::FetchError;
use crate::jwt_keys::{DecodeError, MAX_DOCUMENT_BYTES, userinfo_document};

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
    #[error("Failed to fetch OIDC UserInfo: {source}")]
    Failed { source: UserinfoFailure },
}

// The wrapped steps stay typed, so a caller can tell a transport outage from a status or a decode failure.
#[derive(Debug, Error)]
pub enum UserinfoFailure {
    #[error(transparent)]
    Fetch(#[from] FetchError),
    #[error("OIDC UserInfo endpoint returned status {status}: {text}")]
    Status { status: u16, text: String },
    #[error(transparent)]
    Document(#[from] DecodeError),
    #[error(transparent)]
    Cache(#[from] AuthCacheError),
}

// JWTHandler.get_oidc_userinfo: endpoint check, cache read, one fetch, success log, cache write, then return.
pub async fn fetch_userinfo(
    settings: &JwtVerificationSettings,
    cache: &dyn KeyCache,
    http: &dyn UserinfoHttp,
    token: &str,
) -> Result<IndexMap<String, Json>, UserinfoError> {
    let Some(endpoint) = settings.jwt.userinfo.oidc_userinfo_endpoint.as_deref() else {
        return Err(UserinfoError::NotConfigured);
    };
    if endpoint.is_empty() {
        return Err(UserinfoError::NotConfigured);
    }
    // oidc_userinfo_<sha256 token hex>, so the bearer credential itself never becomes a cache key
    let key = format!(
        "{USERINFO_CACHE_KEY_PREFIX}{}",
        hex::encode(Sha256::digest(token.as_bytes()))
    );
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
        let reply = http.get(endpoint, &authorization, USERINFO_ACCEPT).await?;
        if reply.status != 200 {
            let text = str::from_utf8(&reply.body).unwrap_or_default();
            return Err(UserinfoFailure::Status {
                status: reply.status,
                text: text.to_owned(),
            });
        }
        let userinfo = userinfo_document(&reply.body)?;
        // a privacy deviation from LiteLLM: the outcome is logged, the identity document never is
        tracing::debug!(target: "LiteLLM Proxy", "Received OIDC UserInfo");
        let stored = Json::Object(userinfo.clone());
        cache
            .write_value(&key, &stored, settings.jwt.userinfo.oidc_userinfo_cache_ttl)
            .await?;
        Ok(userinfo)
    }
    .await;
    match fetched {
        Ok(userinfo) => Ok(userinfo),
        Err(source) => {
            tracing::error!(
                target: "LiteLLM Proxy",
                "Error fetching OIDC UserInfo: {source}"
            );
            Err(UserinfoError::Failed { source })
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
