use std::io;
use std::num::NonZeroUsize;

use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use thiserror::Error;

pub mod authority;
pub mod provider;
pub mod region;
pub mod token;
pub mod transport;

mod tests;

pub const DEFAULT_SCOPE: &str = "https://cognitiveservices.azure.com/.default";
pub const DEFAULT_AUTHORITY_HOST: &str = "login.microsoftonline.com";
pub const PROVIDER_CACHE_CAPACITY: NonZeroUsize = NonZeroUsize::new(128).unwrap();
const OS_ENVIRON_PREFIX: &str = "os.environ/";

#[derive(Debug, Error)]
pub enum ClientSecretError {
    #[error("tenant_id, client_id, and client_secret must be provided")]
    MissingCredentials,
    #[error("client_id should be the id of a Microsoft Entra application")]
    EmptyClientId,
    #[error("secret should be a Microsoft Entra application's client secret")]
    EmptyClientSecret,
    #[error(
        "tenant_id should be the id of a Microsoft Entra tenant's id (also called its 'directory id')"
    )]
    EmptyTenantId,
    #[error(
        "Invalid tenant ID provided. You can locate your tenant ID by following the instructions here: https://learn.microsoft.com/partner-center/find-ids-and-domain-names"
    )]
    InvalidTenantId,
    #[error(
        "'{authority}' is an invalid authority. The value must be a TLS protected (https) URL."
    )]
    InvalidAuthority { authority: String },
    #[error(
        "Your given address ({authority}) should consist of an https url with hostname and a minimum of one segment in a path"
    )]
    AuthorityAddress { authority: String },
    #[error("invalid_instance: The authority you provided, {authority}, is not known")]
    InvalidInstance { authority: String },
    #[error("Unable to get authority configuration for {authority}")]
    AuthorityConfiguration { authority: String },
    #[error("Unable to get authority configuration for {authority}")]
    AuthorityConfigurationJson {
        authority: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("the authority discovery response is not JSON")]
    AuthorityResponse {
        #[source]
        source: serde_json::Error,
    },
    #[error("the authority configuration has no {field}")]
    AuthorityMissingField { field: &'static str },
    #[error("the authority discovery request failed")]
    AuthorityTransport {
        #[source]
        source: io::Error,
    },
    #[error("the authority returned HTTP status {status}")]
    AuthorityStatus { status: u16 },
    #[error("the instance discovery response has no tenant discovery endpoint")]
    InstanceMetadata,
    #[error("Authentication failed: {message}")]
    Authentication { message: String },
    #[error("Authentication failed: {source}")]
    TokenTransport {
        #[source]
        source: io::Error,
    },
    #[error("the token response is not JSON")]
    TokenJson {
        #[source]
        source: serde_json::Error,
    },
    #[error("the token response is missing access_token")]
    TokenAccessTokenMissing,
    #[error("the token response is missing expires_in")]
    TokenExpiresInMissing,
    #[error("the token response has no integer {field}")]
    TokenInteger { field: &'static str },
    #[error("the token response field {field} has the wrong type")]
    TokenFieldType { field: &'static str },
    #[error("the token lifetime overflows the clock")]
    TokenExpiryOverflow,
    #[error("the token provider cache is poisoned")]
    ProviderCachePoisoned,
}

impl ClientSecretError {
    // msal re-raises a ValueError from authority validation but retries any other failure without instance discovery
    #[must_use]
    pub const fn is_authority_value(&self) -> bool {
        matches!(
            self,
            Self::InvalidAuthority { .. }
                | Self::AuthorityAddress { .. }
                | Self::InvalidInstance { .. }
                | Self::AuthorityConfiguration { .. }
                | Self::AuthorityConfigurationJson { .. }
                | Self::AuthorityResponse { .. }
        )
    }
}

pub trait SecretResolver {
    fn get_secret_str(&self, reference: &str) -> Option<SecretString>;
}

#[derive(Debug, Clone, Deserialize)]
pub struct ClientSecretInput {
    pub tenant_id: String,
    pub client_id: String,
    pub client_secret: SecretString,
    #[serde(default)]
    pub scope: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ResolvedCredentials {
    pub tenant_id: String,
    pub client_id: String,
    pub client_secret: SecretString,
    pub scope: String,
}

pub fn resolve_client_secret(
    input: &ClientSecretInput,
    resolver: &dyn SecretResolver,
) -> Result<ResolvedCredentials, ClientSecretError> {
    let tenant_id =
        resolve_text(&input.tenant_id, resolver).ok_or(ClientSecretError::MissingCredentials)?;
    let client_id =
        resolve_text(&input.client_id, resolver).ok_or(ClientSecretError::MissingCredentials)?;
    let client_secret = resolve_secret(&input.client_secret, resolver)
        .ok_or(ClientSecretError::MissingCredentials)?;
    Ok(ResolvedCredentials {
        tenant_id,
        client_id,
        client_secret,
        scope: input
            .scope
            .clone()
            .unwrap_or_else(|| DEFAULT_SCOPE.to_owned()),
    })
}

fn resolve_text(value: &str, resolver: &dyn SecretResolver) -> Option<String> {
    if value.starts_with(OS_ENVIRON_PREFIX) {
        resolver
            .get_secret_str(value)
            .map(|secret| secret.expose_secret().to_owned())
    } else {
        Some(value.to_owned())
    }
}

fn resolve_secret(value: &SecretString, resolver: &dyn SecretResolver) -> Option<SecretString> {
    if value.expose_secret().starts_with(OS_ENVIRON_PREFIX) {
        resolver.get_secret_str(value.expose_secret())
    } else {
        Some(value.clone())
    }
}
