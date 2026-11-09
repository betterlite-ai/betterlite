use std::sync::{Mutex, PoisonError};

use betterlite_http::proxy::transport_error::TransportError;
use chrono::{DateTime, Duration, Utc};
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde_json::value::RawValue;
use thiserror::Error;

use super::credentials::{Credentials, validate_credentials};
use super::error::{SapAuthError, TokenFailure};

pub const GRANT_TYPE: &str = "client_credentials";
pub const DEFAULT_EXPIRY_SECONDS: i64 = 3600;
pub const EXPIRY_BUFFER_MINUTES: i64 = 60;
const WITHHELD: &str = "token response names access_token but is not valid JSON, body withheld";

#[derive(Debug)]
pub struct TokenForm {
    pub grant_type: &'static str,
    pub client_id: String,
    pub client_secret: Option<SecretString>,
}

#[derive(Debug)]
pub enum ClientIdentity {
    None,
    Pem { cert: String, key: SecretString },
    Files { cert_path: String, key_path: String },
}

#[derive(Debug)]
pub struct TokenRequest {
    pub url: String,
    pub form: TokenForm,
    pub identity: ClientIdentity,
}

#[derive(Debug)]
pub struct BearerToken {
    pub header: SecretString,
    pub expires_at: DateTime<Utc>,
}

#[derive(Deserialize)]
struct TokenBody {
    access_token: SecretString,
    #[serde(default = "default_expires_in")]
    expires_in: i64,
}

const fn default_expires_in() -> i64 {
    DEFAULT_EXPIRY_SECONDS
}

#[derive(Debug, Error)]
pub enum TokenTransportError {
    #[error("{0}")]
    Transport(#[source] Box<TransportError>),
    #[error("{0}")]
    Message(String),
}

pub trait SapClock: Send + Sync {
    fn now(&self) -> DateTime<Utc>;
}

pub trait SapTokenTransport: Send + Sync {
    fn post(&self, request: &TokenRequest) -> Result<String, TokenTransportError>;
}

pub fn build_token_request(credentials: &Credentials) -> Result<TokenRequest, SapAuthError> {
    let auth_url = credentials
        .auth_url
        .clone()
        .filter(|url| !url.is_empty())
        .ok_or(SapAuthError::MissingCredentials)?;
    let client_id = credentials
        .client_id
        .clone()
        .filter(|id| !id.is_empty())
        .ok_or(SapAuthError::MissingCredentials)?;
    let client_secret = credentials
        .client_secret
        .clone()
        .filter(|secret| !secret.expose_secret().is_empty());
    let identity = if client_secret.is_some() {
        ClientIdentity::None
    } else {
        choose_cert_identity(credentials)?
    };
    Ok(TokenRequest {
        url: auth_url,
        form: TokenForm {
            grant_type: GRANT_TYPE,
            client_id,
            client_secret,
        },
        identity,
    })
}

fn choose_cert_identity(credentials: &Credentials) -> Result<ClientIdentity, SapAuthError> {
    if let (Some(cert), Some(key)) = (&credentials.cert_str, &credentials.key_str)
        && !cert.is_empty()
        && !key.expose_secret().is_empty()
    {
        return Ok(ClientIdentity::Pem {
            cert: cert.replace("\\n", "\n"),
            key: SecretString::from(key.expose_secret().replace("\\n", "\n")),
        });
    }
    match (&credentials.cert_file_path, &credentials.key_file_path) {
        (Some(cert_path), Some(key_path)) => Ok(ClientIdentity::Files {
            cert_path: cert_path.clone(),
            key_path: key_path.clone(),
        }),
        (None, _) | (_, None) => Err(SapAuthError::IncompleteCredentials),
    }
}

#[derive(Deserialize)]
struct TokenText<'body> {
    #[serde(borrow)]
    access_token: &'body RawValue,
}

// LiteLLM puts the whole response in the error text, so a parseable token must not reach it
fn error_text(body: &str) -> String {
    match serde_json::from_str::<TokenText<'_>>(body) {
        Ok(TokenText { access_token: raw }) if raw.get().starts_with('"') => {
            body.replace(raw.get(), "\"<redacted>\"")
        }
        Err(_) if body.contains("access_token") => WITHHELD.to_owned(),
        _ => body.to_owned(),
    }
}

pub fn parse_token_response(body: &str, now: DateTime<Utc>) -> Result<BearerToken, SapAuthError> {
    let parsed =
        serde_json::from_str::<TokenBody>(body).map_err(|source| SapAuthError::TokenRequest {
            text: error_text(body),
            source: Box::new(TokenFailure::Response(source)),
        })?;
    let expires_at = Duration::try_seconds(parsed.expires_in)
        .and_then(|duration| now.checked_add_signed(duration))
        .ok_or_else(|| SapAuthError::TokenRequest {
            text: error_text(body),
            source: Box::new(TokenFailure::Expiry(parsed.expires_in)),
        })?;
    let header = SecretString::from(format!("Bearer {}", parsed.access_token.expose_secret()));
    Ok(BearerToken { header, expires_at })
}

#[derive(Debug)]
pub struct TokenCache {
    request: TokenRequest,
    cached: Mutex<Option<BearerToken>>,
}

impl TokenCache {
    pub fn new(credentials: &Credentials) -> Result<Self, SapAuthError> {
        validate_credentials(credentials)?;
        let request = build_token_request(credentials)?;
        let cached = Mutex::new(None);
        Ok(Self { request, cached })
    }

    pub fn token(
        &self,
        clock: &dyn SapClock,
        transport: &dyn SapTokenTransport,
    ) -> Result<SecretString, SapAuthError> {
        let now = clock.now();
        let mut cached = self.cached.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(token) = cached.as_ref()
            && token.expires_at.signed_duration_since(now)
                >= Duration::minutes(EXPIRY_BUFFER_MINUTES)
        {
            return Ok(token.header.clone());
        }
        let body = transport
            .post(&self.request)
            .map_err(|source| SapAuthError::TokenRequest {
                text: source.to_string(),
                source: Box::new(TokenFailure::Transport(source)),
            })?;
        let token = parse_token_response(&body, now)?;
        let header = token.header.clone();
        *cached = Some(token);
        drop(cached);
        Ok(header)
    }
}
