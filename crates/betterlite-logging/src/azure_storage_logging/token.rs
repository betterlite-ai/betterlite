use std::sync::Arc;
use std::time::Duration;

use http::Method;
use http::header::CONTENT_TYPE;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use tokio::task::JoinError;
use url::form_urlencoded;

use super::settings::AzureStorageSettings;
use super::transport::{STORAGE_TIMEOUT, StorageRequest, StorageResponse, StorageTransportError};

pub const STORAGE_SCOPE: &str = "https://storage.azure.com/.default";
pub const REFRESH_MARGIN: Duration = Duration::from_secs(300);
// LiteLLM fixes the lifetime at one hour instead of reading expires_in from the token response
pub const TOKEN_LIFETIME: Duration = Duration::from_secs(3600);
pub const DEFAULT_AUTHORITY_HOST: &str = "https://login.microsoftonline.com";
pub const TOKEN_PATH: &str = "/oauth2/v2.0/token";

#[derive(Debug, thiserror::Error)]
pub enum TokenError {
    #[error("Missing required environment variable: AZURE_STORAGE_TENANT_ID")]
    MissingTenantId,
    #[error("Missing required environment variable: AZURE_STORAGE_CLIENT_ID")]
    MissingClientId,
    #[error("Missing required environment variable: AZURE_STORAGE_CLIENT_SECRET")]
    MissingClientSecret,
    #[error("the Azure AD token request could not be completed")]
    Transport(#[from] StorageTransportError),
    #[error("the Azure AD token endpoint answered with status {status}")]
    Status { status: u16 },
    #[error("the Azure AD token response is not JSON")]
    Body(#[source] serde_json::Error),
    #[error("the Azure AD token response has no access_token")]
    MissingAccessToken,
    #[error("the Azure AD token worker task failed")]
    Worker(#[source] JoinError),
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

    pub fn store(&mut self, token: SecretString, now: Duration) {
        self.token = Some(token);
        self.expires_at = Some(now.saturating_add(TOKEN_LIFETIME));
    }
}

// get_azure_ad_token_from_azure_storage raises these three before it calls B673
pub fn credentials(
    settings: &AzureStorageSettings,
) -> Result<(String, String, SecretString), TokenError> {
    let tenant_id = settings
        .tenant_id
        .clone()
        .ok_or(TokenError::MissingTenantId)?;
    let client_id = settings
        .client_id
        .clone()
        .ok_or(TokenError::MissingClientId)?;
    let client_secret = settings
        .client_secret
        .clone()
        .ok_or(TokenError::MissingClientSecret)?;
    Ok((tenant_id, client_id, client_secret))
}

#[must_use]
pub fn client_secret_request(credentials: &(String, String, SecretString)) -> StorageRequest {
    let (tenant_id, client_id, client_secret) = credentials;
    let form = form_urlencoded::Serializer::new(String::new())
        .append_pair("grant_type", "client_credentials")
        .append_pair("client_id", client_id)
        .append_pair("client_secret", client_secret.expose_secret())
        .append_pair("scope", STORAGE_SCOPE)
        .finish();
    StorageRequest {
        method: Method::POST,
        url: format!("{DEFAULT_AUTHORITY_HOST}/{tenant_id}{TOKEN_PATH}"),
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
pub fn parse_token_response(response: &StorageResponse) -> Result<SecretString, TokenError> {
    if response.status != 200 {
        return Err(TokenError::Status {
            status: response.status,
        });
    }
    let parsed: TokenResponse = serde_json::from_slice(&response.body).map_err(TokenError::Body)?;
    parsed.access_token.ok_or(TokenError::MissingAccessToken)
}
