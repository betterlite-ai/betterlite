use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use secrecy::{ExposeSecret as _, SecretString};
use thiserror::Error;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::resolve_string;

use super::{RawSettings, config_or_environment_flag};

const GCP_TOKEN_TTL_SECONDS: u64 = 3300;

#[derive(Debug, Clone)]
pub struct GcpSettings {
    pub service_account: SecretString,
    pub ssl_ca_certs: Option<String>,
    pub token_ttl_seconds: u64,
    pub token_cache: Arc<GcpTokenCache>,
}

pub trait Clock: Send + Sync {
    fn now(&self) -> Instant;
}

#[derive(Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}

#[derive(Debug, Error)]
#[error("{message}")]
pub struct GcpTokenFetchError {
    pub message: String,
}

pub trait GcpTokenFetcher: Send + Sync {
    fn fetch(&self, service_account: &str) -> Result<SecretString, GcpTokenFetchError>;
}

#[derive(Debug, Error)]
pub enum GcpTokenError {
    #[error("failed to generate a GCP IAM access token: {source}")]
    Fetch {
        #[source]
        source: GcpTokenFetchError,
    },
    #[error("the GCP token cache lock was poisoned")]
    LockPoisoned,
    #[error("the GCP IAM token TTL overflows the system clock")]
    Expiry,
}

struct CachedToken {
    token: SecretString,
    expires_at: Instant,
}

#[derive(Default)]
pub struct GcpTokenCache {
    entries: Mutex<HashMap<String, CachedToken>>,
}

impl fmt::Debug for GcpTokenCache {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GcpTokenCache").finish_non_exhaustive()
    }
}

impl GcpTokenCache {
    pub fn get_or_fetch(
        &self,
        service_account: &str,
        ttl: Duration,
        clock: &dyn Clock,
        fetcher: &dyn GcpTokenFetcher,
    ) -> Result<SecretString, GcpTokenError> {
        let now = clock.now();
        if let Some(token) = self.fresh_token(service_account, now)? {
            return Ok(token);
        }
        let mut entries = self
            .entries
            .lock()
            .map_err(|_poisoned| GcpTokenError::LockPoisoned)?;
        if let Some(cached) = entries.get(service_account)
            && now < cached.expires_at
        {
            let token = cached.token.clone();
            drop(entries);
            return Ok(token);
        }
        let token = fetcher
            .fetch(service_account)
            .map_err(|source| GcpTokenError::Fetch { source })?;
        let expires_at = now.checked_add(ttl).ok_or(GcpTokenError::Expiry)?;
        entries.insert(
            service_account.to_owned(),
            CachedToken {
                token: token.clone(),
                expires_at,
            },
        );
        drop(entries);
        Ok(token)
    }

    fn fresh_token(
        &self,
        service_account: &str,
        now: Instant,
    ) -> Result<Option<SecretString>, GcpTokenError> {
        let entries = self
            .entries
            .lock()
            .map_err(|_poisoned| GcpTokenError::LockPoisoned)?;
        Ok(entries
            .get(service_account)
            .filter(|cached| now < cached.expires_at)
            .map(|cached| cached.token.clone()))
    }
}

pub struct GcpIamCredentialProvider {
    pub service_account: SecretString,
    pub ttl: Duration,
    pub cache: Arc<GcpTokenCache>,
}

pub trait GcpCredentialSource {
    fn get_credentials(
        &self,
        clock: &dyn Clock,
        fetcher: &dyn GcpTokenFetcher,
    ) -> Result<SecretString, GcpTokenError>;
}

impl GcpCredentialSource for GcpIamCredentialProvider {
    fn get_credentials(
        &self,
        clock: &dyn Clock,
        fetcher: &dyn GcpTokenFetcher,
    ) -> Result<SecretString, GcpTokenError> {
        self.cache.get_or_fetch(
            self.service_account.expose_secret(),
            self.ttl,
            clock,
            fetcher,
        )
    }
}

pub(super) fn build_gcp<L>(
    settings: &RawSettings,
    lookup: &L,
) -> Result<Option<GcpSettings>, ConfigError>
where
    L: EnvironmentLookup,
{
    let configured_service_account = settings
        .gcp_service_account
        .clone()
        .filter(|value| !value.is_empty())
        .map(|value| {
            resolve_string(
                value,
                "litellm_settings.cache_params.gcp_service_account",
                lookup,
            )
        })
        .transpose()?;
    let gcp_service_account =
        configured_service_account.or_else(|| lookup.lookup("REDIS_GCP_SERVICE_ACCOUNT"));
    let configured_ssl_ca_certs = settings
        .gcp_ssl_ca_certs
        .clone()
        .filter(|value| !value.is_empty())
        .map(|value| {
            resolve_string(
                value,
                "litellm_settings.cache_params.gcp_ssl_ca_certs",
                lookup,
            )
        })
        .transpose()?;
    let gcp_ssl_ca_certs =
        configured_ssl_ca_certs.or_else(|| lookup.lookup("REDIS_GCP_SSL_CA_CERTS"));
    let ssl = config_or_environment_flag(
        settings.ssl.as_ref(),
        lookup,
        "litellm_settings.cache_params.ssl",
        "REDIS_SSL",
    )?;
    Ok(gcp_service_account.map(|account| GcpSettings {
        service_account: SecretString::from(account),
        ssl_ca_certs: gcp_ssl_ca_certs.filter(|_| ssl),
        token_ttl_seconds: GCP_TOKEN_TTL_SECONDS,
        token_cache: Arc::new(GcpTokenCache::default()),
    }))
}
