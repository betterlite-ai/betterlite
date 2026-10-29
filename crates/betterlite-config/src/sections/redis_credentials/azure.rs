use std::sync::Arc;

use secrecy::{ExposeSecret as _, SecretString};
use thiserror::Error;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::resolve_secret;

use super::{RawSettings, non_empty, resolve_string_opt};

pub const AZURE_REDIS_SCOPE: &str = "https://redis.azure.com/.default";

#[derive(Debug, Clone)]
pub struct AzureSettings {
    pub client_id: Option<SecretString>,
    pub tenant_id: Option<SecretString>,
    pub client_secret: Option<SecretString>,
    pub username: Option<String>,
    pub credential: AzureCredential,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AzureCredential {
    Default,
    ManagedIdentity,
    ClientSecret,
}

#[derive(Debug, Clone)]
pub struct AzureToken {
    pub token: String,
}

#[derive(Debug, Error)]
pub enum AzureTokenError {
    #[error("failed to fetch an Azure AD access token: {reason}")]
    Fetch { reason: String },
}

pub trait AzureTokenFetcher: Send + Sync {
    fn fetch_token(
        &self,
        credential: AzureCredential,
        scope: &str,
    ) -> Result<AzureToken, AzureTokenError>;
}

// Holds one fetcher across every call so the SDK credential's own token cache and refresh apply.
pub struct AzureAdCredentialProvider {
    pub credential: AzureCredential,
    pub fetcher: Arc<dyn AzureTokenFetcher>,
    pub username: Option<String>,
}

impl AzureAdCredentialProvider {
    pub fn get_credentials(&self) -> Result<(Option<String>, String), AzureTokenError> {
        let token = self
            .fetcher
            .fetch_token(self.credential, AZURE_REDIS_SCOPE)?;
        Ok((self.username.clone(), token.token))
    }
}

fn resolve_azure_field<L>(
    configured: Option<String>,
    path: &str,
    lookup: &L,
    redis_env: &str,
    azure_env: &str,
) -> Result<Option<String>, ConfigError>
where
    L: EnvironmentLookup,
{
    let resolved = resolve_string_opt(non_empty(configured), path, lookup)?;
    Ok(resolved
        .or_else(|| non_empty(lookup.lookup(redis_env)))
        .or_else(|| non_empty(lookup.lookup(azure_env))))
}

fn resolve_azure_client_secret<L>(
    settings: &RawSettings,
    lookup: &L,
) -> Result<Option<SecretString>, ConfigError>
where
    L: EnvironmentLookup,
{
    let resolved = resolve_secret(
        non_empty(
            settings
                .azure_client_secret
                .as_ref()
                .map(|secret| secret.expose_secret().to_owned()),
        ),
        "litellm_settings.cache_params.azure_client_secret",
        lookup,
    )?;
    Ok(resolved
        .or_else(|| non_empty(lookup.lookup("REDIS_AZURE_CLIENT_SECRET")).map(SecretString::from))
        .or_else(|| non_empty(lookup.lookup("AZURE_CLIENT_SECRET")).map(SecretString::from)))
}

const fn select_credential(
    client_id: Option<&str>,
    tenant_id: Option<&str>,
    client_secret: Option<&SecretString>,
) -> AzureCredential {
    if client_id.is_some() && tenant_id.is_some() && client_secret.is_some() {
        AzureCredential::ClientSecret
    } else if client_id.is_some() {
        AzureCredential::ManagedIdentity
    } else {
        AzureCredential::Default
    }
}

pub(super) fn resolve_azure<L>(
    settings: &RawSettings,
    lookup: &L,
) -> Result<AzureSettings, ConfigError>
where
    L: EnvironmentLookup,
{
    let client_id = resolve_azure_field(
        settings.azure_client_id.clone(),
        "litellm_settings.cache_params.azure_client_id",
        lookup,
        "REDIS_AZURE_CLIENT_ID",
        "AZURE_CLIENT_ID",
    )?;
    let tenant_id = resolve_azure_field(
        settings.azure_tenant_id.clone(),
        "litellm_settings.cache_params.azure_tenant_id",
        lookup,
        "REDIS_AZURE_TENANT_ID",
        "AZURE_TENANT_ID",
    )?;
    let client_secret = resolve_azure_client_secret(settings, lookup)?;
    let username = lookup
        .lookup("REDIS_USERNAME")
        .filter(|value| !value.is_empty());
    let credential = select_credential(
        client_id.as_deref(),
        tenant_id.as_deref(),
        client_secret.as_ref(),
    );
    Ok(AzureSettings {
        credential,
        client_id: client_id.map(SecretString::from),
        tenant_id: tenant_id.map(SecretString::from),
        client_secret,
        username,
    })
}
