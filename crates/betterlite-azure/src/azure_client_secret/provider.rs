use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard};

use betterlite_config::sections::azure_client_secret::Settings;
use lru::LruCache;
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
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    token: Option<AccessToken>,
    token_endpoint: Option<String>,
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
        let cached = self.state()?.token.clone();
        if let Some(token) = &cached
            && now < token.refresh_on.unwrap_or(token.expires_on)
        {
            return Ok(token.clone());
        }
        match self.acquire() {
            Ok(token) => {
                self.state()?.token = Some(token.clone());
                Ok(token)
            }
            // msal serves the aging token when the IdP rejects the refresh, other failures raise
            Err(ClientSecretError::TokenAccessTokenMissing) => match cached {
                Some(token) if now < token.expires_on => Ok(token),
                _ => Err(ClientSecretError::TokenAccessTokenMissing),
            },
            Err(error) => Err(error),
        }
    }

    fn state(&self) -> Result<MutexGuard<'_, State>, ClientSecretError> {
        self.state
            .lock()
            .map_err(|_error| ClientSecretError::ProviderCachePoisoned)
    }

    fn acquire(&self) -> Result<AccessToken, ClientSecretError> {
        let known = self.state()?.token_endpoint.clone();
        let token_endpoint = if let Some(endpoint) = known {
            endpoint
        } else {
            let endpoint = self.resolve_token_endpoint()?;
            self.state()?.token_endpoint = Some(endpoint.clone());
            endpoint
        };
        token::request_token(
            &*self.transport,
            &*self.clock,
            &token_endpoint,
            &self.credentials,
        )
    }

    // msal builds its application, region and authority included, once per credential
    fn resolve_token_endpoint(&self) -> Result<String, ClientSecretError> {
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
        Ok(endpoint.token_endpoint)
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
    entries: LruCache<CacheKey, Arc<ClientSecretProvider>>,
}

impl ProviderCache {
    #[must_use]
    pub fn new(transport: Arc<dyn Transport>, clock: Arc<dyn Clock>, settings: Settings) -> Self {
        Self {
            transport,
            clock,
            settings,
            entries: LruCache::new(PROVIDER_CACHE_CAPACITY),
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
        if let Some(provider) = self.entries.get(&key) {
            return Ok(Arc::clone(provider));
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
            state: Mutex::new(State::default()),
        });
        self.entries.put(key, Arc::clone(&provider));
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
