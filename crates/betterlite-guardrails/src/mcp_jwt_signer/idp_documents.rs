pub mod document;

mod tests;

use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use betterlite_http::proxy::limits::{ReadError, read_bounded};
use jsonwebtoken::jwk::Jwk;
use reqwest::header::ACCEPT;
use reqwest::{Client, StatusCode};
use secrecy::{ExposeSecret as _, SecretString};
use thiserror::Error;

use crate::mcp_jwt_signer::idp_documents::document::{
    DiscoveryDocument, IntrospectionResponse, JwksDocument,
};
use crate::mcp_token_claims::Claims;

const JWKS_CACHE_SECONDS: i64 = 3600;
const DISCOVERY_CACHE_SECONDS: i64 = 86_400;
const MAX_DOCUMENT_BYTES: usize = 1024 * 1024;
const ACCEPT_JSON: &str = "application/json";

pub struct IdpClient {
    pub(crate) client: Client,
    pub(crate) timeout: Duration,
    pub(crate) jwks_cache: Mutex<BTreeMap<String, Cached<Vec<Jwk>>>>,
    pub(crate) discovery_cache: Mutex<BTreeMap<String, Cached<DiscoveryDocument>>>,
}

pub(crate) struct Cached<T> {
    value: T,
    fetched_at: i64,
}

#[derive(Debug, Error)]
pub enum IdpError {
    #[error("the request to {uri} failed: {source}")]
    Request {
        uri: String,
        #[source]
        source: reqwest::Error,
    },
    #[error("the request to {uri} returned HTTP {status}")]
    Status { uri: String, status: StatusCode },
    #[error("the response from {uri} is larger than {limit} bytes")]
    TooLarge { uri: String, limit: usize },
    #[error("the response from {uri} could not be read: {source}")]
    Read {
        uri: String,
        #[source]
        source: reqwest::Error,
    },
    #[error("the response from {uri} is not a valid document: {source}")]
    Decode {
        uri: String,
        #[source]
        source: serde_json::Error,
    },
    #[error(
        "token_introspection_endpoint is required for opaque token verification but is not configured"
    )]
    MissingIntrospectionEndpoint,
    #[error("incoming token is inactive (introspection returned active=false)")]
    InactiveToken,
}

impl IdpClient {
    pub async fn fetch_jwks(&self, uri: &str, now: i64) -> Result<Vec<Jwk>, IdpError> {
        if let Some(keys) = cached(&self.jwks_cache, uri, now, JWKS_CACHE_SECONDS) {
            return Ok(keys);
        }
        let body = self.get_bytes(uri).await?;
        let document: JwksDocument =
            serde_json::from_slice(&body).map_err(|source| IdpError::Decode {
                uri: uri.to_owned(),
                source,
            })?;
        let keys = document.keys.unwrap_or_default();
        self.jwks_cache
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(
                uri.to_owned(),
                Cached {
                    value: keys.clone(),
                    fetched_at: now,
                },
            );
        Ok(keys)
    }

    pub async fn fetch_oidc_discovery(&self, uri: &str) -> Result<DiscoveryDocument, IdpError> {
        let body = self.get_bytes(uri).await?;
        let document: DiscoveryDocument =
            serde_json::from_slice(&body).map_err(|source| IdpError::Decode {
                uri: uri.to_owned(),
                source,
            })?;
        Ok(document)
    }

    pub async fn get_oidc_discovery(
        &self,
        uri: Option<&str>,
        now: i64,
    ) -> Result<DiscoveryDocument, IdpError> {
        let Some(discovery_uri) = uri else {
            return Ok(DiscoveryDocument { jwks_uri: None });
        };
        if let Some(document) = cached(
            &self.discovery_cache,
            discovery_uri,
            now,
            DISCOVERY_CACHE_SECONDS,
        ) {
            return Ok(document);
        }
        let document = self.fetch_oidc_discovery(discovery_uri).await?;
        if document
            .jwks_uri
            .as_deref()
            .is_some_and(|jwks_uri| !jwks_uri.is_empty())
        {
            self.discovery_cache
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .insert(
                    discovery_uri.to_owned(),
                    Cached {
                        value: document.clone(),
                        fetched_at: now,
                    },
                );
        }
        Ok(document)
    }

    pub async fn introspect(
        &self,
        endpoint: Option<&str>,
        token: &SecretString,
    ) -> Result<Claims, IdpError> {
        let Some(introspection_endpoint) = endpoint else {
            return Err(IdpError::MissingIntrospectionEndpoint);
        };
        let request = self
            .client
            .post(introspection_endpoint)
            .header(ACCEPT, ACCEPT_JSON)
            .form(&[("token", token.expose_secret())])
            .timeout(self.timeout);
        let response = request.send().await.map_err(|source| IdpError::Request {
            uri: introspection_endpoint.to_owned(),
            source,
        })?;
        let status = response.status();
        if !status.is_success() {
            return Err(IdpError::Status {
                uri: introspection_endpoint.to_owned(),
                status,
            });
        }
        let mut answer = response;
        let body = read_bounded(&mut answer, MAX_DOCUMENT_BYTES)
            .await
            .map_err(|source| read_error(introspection_endpoint, source))?;
        let document: IntrospectionResponse =
            serde_json::from_slice(&body).map_err(|source| IdpError::Decode {
                uri: introspection_endpoint.to_owned(),
                source,
            })?;
        if document.active != Some(true) {
            return Err(IdpError::InactiveToken);
        }
        Ok(document.into_claims())
    }

    async fn get_bytes(&self, uri: &str) -> Result<Vec<u8>, IdpError> {
        let request = self
            .client
            .get(uri)
            .header(ACCEPT, ACCEPT_JSON)
            .timeout(self.timeout);
        let response = request.send().await.map_err(|source| IdpError::Request {
            uri: uri.to_owned(),
            source,
        })?;
        let status = response.status();
        if !status.is_success() {
            return Err(IdpError::Status {
                uri: uri.to_owned(),
                status,
            });
        }
        let mut answer = response;
        read_bounded(&mut answer, MAX_DOCUMENT_BYTES)
            .await
            .map_err(|source| read_error(uri, source))
    }
}

fn cached<T>(
    cache: &Mutex<BTreeMap<String, Cached<T>>>,
    uri: &str,
    now: i64,
    ttl_seconds: i64,
) -> Option<T>
where
    T: Clone,
{
    let entry = cache
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(uri)
        .map(|entry| (entry.value.clone(), entry.fetched_at));
    let (value, fetched_at) = entry?;
    now.checked_sub(fetched_at)
        .is_some_and(|age| age < ttl_seconds)
        .then_some(value)
}

fn read_error(uri: &str, failure: ReadError) -> IdpError {
    match failure {
        ReadError::TooLarge { limit } => IdpError::TooLarge {
            uri: uri.to_owned(),
            limit,
        },
        ReadError::Failed(source) => IdpError::Read {
            uri: uri.to_owned(),
            source,
        },
    }
}
