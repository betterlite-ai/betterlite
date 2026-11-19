use http::header::HeaderValue;
use secrecy::{ExposeSecret as _, SecretString};

use crate::credentials::source::TokenReply;
use crate::endpoints::projection::TokenProvider;

use crate::azure_files::error::AzureFilesError;

// The three ways the SDK clients authenticate, plus the client built without any credential
#[derive(Clone)]
pub enum FileAuth {
    ApiKey(SecretString),
    Bearer(SecretString),
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthKind {
    ApiKey,
    Bearer,
    None,
}

pub struct AuthHeader {
    pub(super) kind: AuthKind,
    pub(super) value: HeaderValue,
}

impl FileAuth {
    pub(super) fn secret(&self) -> SecretString {
        match self {
            Self::ApiKey(secret) | Self::Bearer(secret) => secret.clone(),
            Self::None => SecretString::from(String::new()),
        }
    }

    pub(super) fn header(&self) -> Result<Option<AuthHeader>, AzureFilesError> {
        let (kind, raw) = match self {
            Self::ApiKey(secret) => (AuthKind::ApiKey, secret.expose_secret().to_owned()),
            Self::Bearer(token) => (
                AuthKind::Bearer,
                format!("Bearer {}", token.expose_secret()),
            ),
            Self::None => return Ok(None),
        };
        let mut value =
            HeaderValue::from_str(&raw).map_err(|source| AzureFilesError::Header { source })?;
        value.set_sensitive(true);
        Ok(Some(AuthHeader { kind, value }))
    }
}

pub(super) fn provider_auth(provider: &TokenProvider<'_>) -> Result<FileAuth, AzureFilesError> {
    let reply = provider
        .0
        .token()
        .map_err(|failure| AzureFilesError::TokenProvider {
            class: failure.class,
            message: failure.message,
        })?;
    match reply {
        // the SDK refuses a provider that answers with anything but a non-empty string
        TokenReply::Text(text) if !text.expose_secret().is_empty() => Ok(FileAuth::Bearer(text)),
        TokenReply::Text(_) => Err(AzureFilesError::TokenNotString {
            returned: String::new(),
        }),
        TokenReply::NonText { type_name } => Err(AzureFilesError::TokenNotString {
            returned: type_name,
        }),
    }
}

// LiteLLM's credential order: the ad token, then its provider, then the api key
pub(super) fn resolve_auth(
    ad_token: Option<SecretString>,
    provider: Option<&TokenProvider<'_>>,
    api_key: Option<SecretString>,
) -> Result<FileAuth, AzureFilesError> {
    if let Some(token) = ad_token {
        return Ok(FileAuth::Bearer(token));
    }
    if let Some(provider) = provider {
        return provider_auth(provider);
    }
    match api_key {
        // the SDK raises this before it looks at the base URL or the version
        None => Err(AzureFilesError::MissingCredentials),
        // an explicit empty key builds the SDK client, which fails per request; this port fails earlier
        Some(key) if key.expose_secret().is_empty() => Err(AzureFilesError::UnableToHandleAuth),
        Some(key) => Ok(FileAuth::ApiKey(key)),
    }
}
