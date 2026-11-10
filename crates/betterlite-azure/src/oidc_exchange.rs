use std::collections::BTreeMap;
use std::fmt;
use std::io;
use std::sync::{Arc, Mutex, MutexGuard};

use betterlite_config::sections::azure_oidc_exchange::Settings;
use betterlite_context::local_ttl_cache::credential::{
    Credential, CredentialHandle, CredentialIdentity,
};
use betterlite_context::local_ttl_cache::{CacheError, CacheValue, LocalTtlCache};
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use sha2::{Digest as _, Sha256};
use thiserror::Error;

use crate::azure_client_secret::SecretResolver;
use token::decode_azure_oidc_token;
use transport::{Clock, Transport};

pub mod token;
pub mod transport;

mod tests;

pub const DEFAULT_SCOPE: &str = "https://cognitiveservices.azure.com/.default";
pub const CLIENT_ASSERTION_TYPE: &str = "urn:ietf:params:oauth:client-assertion-type:jwt-bearer";
const CLIENT_ID_ENV: &str = "AZURE_CLIENT_ID";
const TENANT_ID_ENV: &str = "AZURE_TENANT_ID";
const CACHE_CAPACITY: i64 = 200;
const CACHE_ITEM_KIB: i64 = 1024;

#[derive(Debug, Error)]
pub enum OidcExchangeError {
    #[error("AZURE_CLIENT_ID and AZURE_TENANT_ID must be set")]
    MissingClientTenant,
    #[error("OIDC token could not be retrieved from secret manager.")]
    UnresolvedOidcToken,
    #[error("Azure AD Token access_token not returned")]
    AccessTokenMissing,
    #[error("Azure AD Token expires_in not returned")]
    ExpiresInMissing,
    #[error("the token response is not JSON")]
    TokenJson {
        #[source]
        source: serde_json::Error,
    },
    #[error("the token response field {field} has the wrong type")]
    TokenFieldType { field: &'static str },
    #[error("the token request failed")]
    TokenTransport {
        #[source]
        source: io::Error,
    },
    #[error("{message}")]
    TokenStatus { status: u16, message: String },
    #[error("the token cache is unavailable")]
    Cache {
        #[source]
        source: CacheError,
    },
    #[error("the token cache is poisoned")]
    CachePoisoned,
}

impl OidcExchangeError {
    #[must_use]
    pub const fn status_code(&self) -> Option<u16> {
        match self {
            Self::MissingClientTenant | Self::AccessTokenMissing | Self::ExpiresInMissing => {
                Some(422)
            }
            Self::UnresolvedOidcToken => Some(401),
            Self::TokenStatus { status, .. } => Some(*status),
            Self::TokenJson { .. }
            | Self::TokenFieldType { .. }
            | Self::TokenTransport { .. }
            | Self::Cache { .. }
            | Self::CachePoisoned => None,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct OidcExchangeInput {
    pub azure_ad_token: SecretString,
    #[serde(default)]
    pub azure_client_id: Option<String>,
    #[serde(default)]
    pub azure_tenant_id: Option<String>,
    #[serde(default)]
    pub scope: Option<String>,
}

pub struct AzureOidcExchange {
    settings: Settings,
    environment: BTreeMap<String, String>,
    transport: Arc<dyn Transport>,
    cache: Mutex<LocalTtlCache>,
}

// the environment can hold the assertion's value, so Debug names only the configured authority
impl fmt::Debug for AzureOidcExchange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AzureOidcExchange")
            .field("authority_host", &self.settings.azure_authority_host)
            .finish_non_exhaustive()
    }
}

struct ExchangeRequest {
    client_id: String,
    tenant_id: String,
    scope: String,
    oidc_token: SecretString,
    cache_key: String,
}

impl AzureOidcExchange {
    pub fn new(
        settings: Settings,
        environment: BTreeMap<String, String>,
        transport: Arc<dyn Transport>,
        clock: Arc<dyn Clock>,
    ) -> Result<Self, OidcExchangeError> {
        let cache = LocalTtlCache::with_clock(
            Some(CACHE_CAPACITY),
            None,
            None,
            CACHE_ITEM_KIB,
            Arc::new(move || clock.now_unix_seconds()),
        )
        .map_err(|source| OidcExchangeError::Cache { source })?;
        Ok(Self {
            settings,
            environment,
            transport,
            cache: Mutex::new(cache),
        })
    }

    pub fn token(
        &self,
        input: &OidcExchangeInput,
        resolver: &dyn SecretResolver,
    ) -> Result<SecretString, OidcExchangeError> {
        let request = self.request(input, resolver)?;
        if let Some(token) = self.cached(&request.cache_key)? {
            return Ok(token);
        }
        self.fetch(&request)
    }

    fn request(
        &self,
        input: &OidcExchangeInput,
        resolver: &dyn SecretResolver,
    ) -> Result<ExchangeRequest, OidcExchangeError> {
        let client_id = self
            .identity(input.azure_client_id.as_deref(), CLIENT_ID_ENV)
            .ok_or(OidcExchangeError::MissingClientTenant)?;
        let tenant_id = self
            .identity(input.azure_tenant_id.as_deref(), TENANT_ID_ENV)
            .ok_or(OidcExchangeError::MissingClientTenant)?;
        let scope = input.scope.as_deref().unwrap_or(DEFAULT_SCOPE);
        let oidc_token = resolver
            .get_secret_str(input.azure_ad_token.expose_secret())
            .ok_or(OidcExchangeError::UnresolvedOidcToken)?;
        let cache_key = cache_key(
            &self.settings.azure_authority_host,
            &tenant_id,
            &client_id,
            scope,
            &oidc_token,
        );
        Ok(ExchangeRequest {
            client_id,
            tenant_id,
            scope: scope.to_owned(),
            oidc_token,
            cache_key,
        })
    }

    fn cached(&self, cache_key: &str) -> Result<Option<SecretString>, OidcExchangeError> {
        let cached = self
            .lock_cache()?
            .get(cache_key)
            .map_err(|source| OidcExchangeError::Cache { source })?;
        Ok(match cached {
            Some(CacheValue::Credential(handle)) => Some(handle.0.token.clone()),
            // any other variant would mean a writer other than this module filled the cache
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

    fn fetch(&self, request: &ExchangeRequest) -> Result<SecretString, OidcExchangeError> {
        let url = format!(
            "{}/{}/oauth2/v2.0/token",
            self.settings.azure_authority_host, request.tenant_id
        );
        let reply = self
            .transport
            .post_form(
                &url,
                &assertion_form(&request.client_id, &request.scope, &request.oidc_token),
            )
            .map_err(|source| OidcExchangeError::TokenTransport { source })?;
        if reply.status != 200 {
            return Err(OidcExchangeError::TokenStatus {
                status: reply.status,
                message: reply.body.expose_secret().to_owned(),
            });
        }
        let decoded = decode_azure_oidc_token(reply.body.expose_secret().as_bytes())?;
        let access_token = decoded
            .access_token
            .ok_or(OidcExchangeError::AccessTokenMissing)?;
        let expires_in = decoded
            .expires_in
            .ok_or(OidcExchangeError::ExpiresInMissing)?;
        let ttl = serde_json::Number::from(expires_in).as_f64();
        let entry = CacheValue::Credential(CredentialHandle(Arc::new(Credential {
            token: access_token.clone(),
            headers: BTreeMap::new(),
        })));
        self.lock_cache()?
            .set(&request.cache_key, entry, ttl)
            .map_err(|source| OidcExchangeError::Cache { source })?;
        Ok(access_token)
    }

    fn identity(&self, explicit: Option<&str>, environment_name: &str) -> Option<String> {
        match explicit {
            Some(value) if !value.is_empty() => Some(value.to_owned()),
            // LiteLLM falls back to the environment for an empty explicit value and keeps an empty one
            _ => self.environment.get(environment_name).cloned(),
        }
    }

    fn lock_cache(&self) -> Result<MutexGuard<'_, LocalTtlCache>, OidcExchangeError> {
        self.cache
            .lock()
            .map_err(|_error| OidcExchangeError::CachePoisoned)
    }
}

fn assertion_form(
    client_id: &str,
    scope: &str,
    oidc_token: &SecretString,
) -> Vec<(String, SecretString)> {
    vec![
        ("client_id".to_owned(), SecretString::from(client_id)),
        (
            "grant_type".to_owned(),
            SecretString::from("client_credentials"),
        ),
        ("scope".to_owned(), SecretString::from(scope)),
        (
            "client_assertion_type".to_owned(),
            SecretString::from(CLIENT_ASSERTION_TYPE),
        ),
        ("client_assertion".to_owned(), oidc_token.clone()),
    ]
}

fn cache_key(
    authority: &str,
    tenant: &str,
    client: &str,
    scope: &str,
    token: &SecretString,
) -> String {
    let identity = CredentialIdentity {
        authority: authority.to_owned(),
        principal: client.to_owned(),
    };
    let digest = hex::encode(Sha256::digest(token.expose_secret().as_bytes()));
    // the digest is a fixed 64 hex digits, so the tenant length prefix keeps the scope boundary exact
    identity.cache_key(&format!("{}:{tenant}:{digest}{scope}", tenant.len()))
}
