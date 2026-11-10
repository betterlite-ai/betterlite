use std::fmt;
use std::sync::{Arc, Mutex};

use betterlite_config::sections::azure_client_secret::Settings;
use indexmap::IndexMap;
use secrecy::ExposeSecret as _;
use sha2::{Digest as _, Sha256};

use super::authority::{self, Authority};
use super::region;
use super::token::{self, AccessToken};
use super::transport::{Clock, Transport};
use super::{
    ClientSecretError, ClientSecretInput, DEFAULT_AUTHORITY_HOST, PROVIDER_CACHE_CAPACITY,
    ResolvedCredentials, SecretResolver, resolve_client_secret,
};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct CacheKey {
    tenant_id: String,
    client_id: String,
    secret_digest: String,
    scope: String,
}

impl CacheKey {
    fn new(credentials: &ResolvedCredentials) -> Self {
        Self {
            tenant_id: credentials.tenant_id.clone(),
            client_id: credentials.client_id.clone(),
            secret_digest: hex::encode(Sha256::digest(
                credentials.client_secret.expose_secret().as_bytes(),
            )),
            scope: credentials.scope.clone(),
        }
    }
}

pub struct ClientSecretProvider {
    credentials: ResolvedCredentials,
    authority_host: String,
    settings: Settings,
    transport: Arc<dyn Transport>,
    clock: Arc<dyn Clock>,
    token: Mutex<Option<AccessToken>>,
}

// the credential's secret and every acquired token stay out of the Debug output
impl fmt::Debug for ClientSecretProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ClientSecretProvider")
            .field("tenant_id", &self.credentials.tenant_id)
            .field("client_id", &self.credentials.client_id)
            .field("scope", &self.credentials.scope)
            .field("authority_host", &self.authority_host)
            .finish_non_exhaustive()
    }
}

impl ClientSecretProvider {
    pub fn token(&self) -> Result<AccessToken, ClientSecretError> {
        let now = self.clock.now_unix_seconds();
        if let Some(token) = self.cached(now)? {
            return Ok(token);
        }
        let token = self.acquire()?;
        let mut cached = self
            .token
            .lock()
            .map_err(|_error| ClientSecretError::ProviderCachePoisoned)?;
        *cached = Some(token.clone());
        drop(cached);
        Ok(token)
    }

    fn cached(&self, now: i64) -> Result<Option<AccessToken>, ClientSecretError> {
        let cached = self
            .token
            .lock()
            .map_err(|_error| ClientSecretError::ProviderCachePoisoned)?;
        let Some(token) = cached.as_ref() else {
            return Ok(None);
        };
        let refresh_at = token.refresh_on.unwrap_or(token.expires_on);
        let reuse = (now < refresh_at).then(|| token.clone());
        drop(cached);
        Ok(reuse)
    }

    fn acquire(&self) -> Result<AccessToken, ClientSecretError> {
        let selection = region::select_region(&self.settings, &*self.transport);
        let central = self.central_authority(selection.opted_in)?;
        let endpoint = match selection
            .region
            .as_deref()
            .filter(|region| !region.is_empty())
        {
            Some(region) => authority::regional_authority(&*self.transport, &central, region)?,
            None => central,
        };
        token::request_token(
            &*self.transport,
            &*self.clock,
            &endpoint.token_endpoint,
            &self.credentials,
        )
    }

    fn central_authority(&self, opted_in: bool) -> Result<Authority, ClientSecretError> {
        let discover = |instance_discovery| {
            authority::discover_authority(
                &*self.transport,
                &self.authority_host,
                &self.credentials.tenant_id,
                instance_discovery,
            )
        };
        match discover(true) {
            Ok(authority) => Ok(authority),
            Err(error) if opted_in && !error.is_authority_value() => discover(false),
            Err(error) => Err(error),
        }
    }
}

pub struct ProviderCache {
    transport: Arc<dyn Transport>,
    clock: Arc<dyn Clock>,
    settings: Settings,
    entries: IndexMap<CacheKey, Arc<ClientSecretProvider>>,
}

impl ProviderCache {
    #[must_use]
    pub fn new(transport: Arc<dyn Transport>, clock: Arc<dyn Clock>, settings: Settings) -> Self {
        Self {
            transport,
            clock,
            settings,
            entries: IndexMap::new(),
        }
    }

    pub fn provider(
        &mut self,
        input: &ClientSecretInput,
        resolver: &dyn SecretResolver,
        authority_host: Option<&str>,
    ) -> Result<Arc<ClientSecretProvider>, ClientSecretError> {
        let credentials = resolve_client_secret(input, resolver)?;
        let key = CacheKey::new(&credentials);
        if let Some(provider) = self.entries.shift_remove(&key) {
            self.entries.insert(key, Arc::clone(&provider));
            return Ok(provider);
        }
        let normalized_host = validate_credentials(
            &credentials,
            authority_host.unwrap_or(DEFAULT_AUTHORITY_HOST),
        )?;
        let provider = Arc::new(ClientSecretProvider {
            credentials,
            authority_host: normalized_host,
            settings: self.settings.clone(),
            transport: Arc::clone(&self.transport),
            clock: Arc::clone(&self.clock),
            token: Mutex::new(None),
        });
        self.entries.insert(key, Arc::clone(&provider));
        while self.entries.len() > PROVIDER_CACHE_CAPACITY {
            drop(self.entries.shift_remove_index(0));
        }
        Ok(provider)
    }
}

fn validate_credentials(
    credentials: &ResolvedCredentials,
    authority_host: &str,
) -> Result<String, ClientSecretError> {
    if credentials.client_id.is_empty() {
        return Err(ClientSecretError::EmptyClientId);
    }
    if credentials.client_secret.expose_secret().is_empty() {
        return Err(ClientSecretError::EmptyClientSecret);
    }
    if credentials.tenant_id.is_empty() {
        return Err(ClientSecretError::EmptyTenantId);
    }
    let normalized = authority::normalize_authority(authority_host)?;
    let valid = credentials
        .tenant_id
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '.'));
    if !valid {
        return Err(ClientSecretError::InvalidTenantId);
    }
    Ok(normalized)
}
