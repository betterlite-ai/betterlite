use std::sync::Arc;
use std::time::Duration;

use betterlite_config::loader::EnvironmentLookup;
use http::Method;
use http::header::CONTENT_TYPE;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use url::form_urlencoded;

use super::settings::AzureStorageSettings;
use super::transport::{STORAGE_TIMEOUT, StorageRequest, StorageTransport};

pub const STORAGE_SCOPE: &str = "https://storage.azure.com/.default";
pub const REFRESH_MARGIN: Duration = Duration::from_secs(300);
// LiteLLM fixes the lifetime at one hour instead of reading expires_in from the token response
pub const TOKEN_LIFETIME: Duration = Duration::from_secs(3600);
pub const DEFAULT_AUTHORITY_HOST: &str = "https://login.microsoftonline.com";
const OS_ENVIRON_PREFIX: &str = "os.environ/";
pub const TOKEN_PATH: &str = "/oauth2/v2.0/token";

#[derive(Debug, thiserror::Error)]
pub enum TokenError {
    #[error("Missing required environment variable: AZURE_STORAGE_TENANT_ID")]
    MissingTenantId,
    #[error("Missing required environment variable: AZURE_STORAGE_CLIENT_ID")]
    MissingClientId,
    #[error("Missing required environment variable: AZURE_STORAGE_CLIENT_SECRET")]
    MissingClientSecret,
    #[error("tenant_id, client_id, and client_secret must be provided")]
    MissingCredentials,
    #[error("the Azure AD token request could not be completed")]
    Transport(#[from] super::transport::StorageTransportError),
    #[error("the Azure AD token endpoint answered with status {status}")]
    Status { status: u16, detail: String },
    #[error("the Azure AD token response is not JSON")]
    Body(#[source] serde_json::Error),
    #[error("the Azure AD token response has no access_token")]
    MissingAccessToken,
    #[error("the Azure AD token worker task failed")]
    Worker(#[source] tokio::task::JoinError),
    #[error(transparent)]
    Chain(#[from] ChainError),
}

#[derive(Debug, thiserror::Error)]
pub enum ChainError {
    #[error("the Azure credential chain has no available credential")]
    Unavailable,
    #[error("the Azure credential chain failed: {message}")]
    Authentication { message: String },
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: Option<SecretString>,
}

// the deployment identity chain is a blocking SDK walk, so the logger calls this on a worker thread
pub trait ChainTokenProvider: Send + Sync {
    fn token(&self) -> Result<SecretString, ChainError>;
}

pub trait ChainProviderFactory: Send + Sync {
    fn provider(&self) -> Result<Arc<dyn ChainTokenProvider>, ChainError>;
}

#[derive(Debug, Default)]
pub struct TokenState {
    pub token: Option<SecretString>,
    pub expires_at: Option<Duration>,
}

impl TokenState {
    #[must_use]
    pub fn is_expired(&self, now: Duration) -> bool {
        self.token
            .as_ref()
            .zip(self.expires_at)
            .is_some_and(|(_token, expires_at)| now.saturating_add(REFRESH_MARGIN) >= expires_at)
    }

    #[must_use]
    pub fn cached(&self, now: Duration) -> Option<SecretString> {
        if self.is_expired(now) {
            return None;
        }
        self.token.clone()
    }

    pub fn store(&mut self, token: SecretString, ttl: Duration, now: Duration) {
        self.token = Some(token);
        self.expires_at = Some(now.saturating_add(ttl));
    }
}

fn resolve_text(raw: &str, environment: &dyn EnvironmentLookup) -> Option<String> {
    raw.strip_prefix(OS_ENVIRON_PREFIX)
        .map_or_else(|| Some(raw.to_owned()), |name| environment.lookup(name))
}

pub struct EntraCredentials {
    pub tenant_id: String,
    pub client_id: String,
    pub client_secret: SecretString,
}

// the same three missing-variable messages get_azure_ad_token_from_azure_storage raises before B673 resolves them
pub fn resolve_entra_credentials(
    settings: &AzureStorageSettings,
    environment: &dyn EnvironmentLookup,
) -> Result<EntraCredentials, TokenError> {
    let tenant_id = settings
        .tenant_id
        .as_deref()
        .ok_or(TokenError::MissingTenantId)?;
    let client_id = settings
        .client_id
        .as_deref()
        .ok_or(TokenError::MissingClientId)?;
    let client_secret = settings
        .client_secret
        .as_ref()
        .ok_or(TokenError::MissingClientSecret)?;
    let resolved_tenant =
        resolve_text(tenant_id, environment).ok_or(TokenError::MissingCredentials)?;
    let resolved_client =
        resolve_text(client_id, environment).ok_or(TokenError::MissingCredentials)?;
    let resolved_secret = client_secret
        .expose_secret()
        .strip_prefix(OS_ENVIRON_PREFIX)
        .map_or_else(
            || Some(SecretString::from(client_secret.expose_secret())),
            |name| environment.lookup(name).map(SecretString::from),
        )
        .ok_or(TokenError::MissingCredentials)?;
    Ok(EntraCredentials {
        tenant_id: resolved_tenant,
        client_id: resolved_client,
        client_secret: resolved_secret,
    })
}

#[must_use]
pub fn client_secret_request(credentials: &EntraCredentials) -> StorageRequest {
    let form = form_urlencoded::Serializer::new(String::new())
        .append_pair("grant_type", "client_credentials")
        .append_pair("client_id", &credentials.client_id)
        .append_pair("client_secret", credentials.client_secret.expose_secret())
        .append_pair("scope", STORAGE_SCOPE)
        .finish();
    StorageRequest {
        method: Method::POST,
        url: format!(
            "{DEFAULT_AUTHORITY_HOST}/{}{TOKEN_PATH}",
            credentials.tenant_id
        ),
        query: Vec::new(),
        headers: vec![(
            CONTENT_TYPE.as_str().to_owned(),
            "application/x-www-form-urlencoded".to_owned(),
        )],
        body: form.into_bytes(),
        deadline: STORAGE_TIMEOUT,
    }
}

// a token response that cannot be read raises in the SDK, so the upload fails rather than sending an empty bearer
pub fn parse_token_response(
    response: &super::transport::StorageResponse,
) -> Result<SecretString, TokenError> {
    if response.status != 200 {
        return Err(TokenError::Status {
            status: response.status,
            detail: String::from_utf8_lossy(&response.body).into_owned(),
        });
    }
    let parsed: TokenResponse = serde_json::from_slice(&response.body).map_err(TokenError::Body)?;
    parsed.access_token.ok_or(TokenError::MissingAccessToken)
}

pub async fn fetch_client_secret_token(
    transport: &dyn StorageTransport,
    credentials: &EntraCredentials,
) -> Result<SecretString, TokenError> {
    let request = client_secret_request(credentials);
    let response = transport.send(request).await?;
    parse_token_response(&response)
}
