use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Instant;

use secrecy::SecretString;
use thiserror::Error;

use crate::oidc::http_timeouts::OidcHttpTimeouts;

pub type OidcTokenCache = Mutex<HashMap<String, (SecretString, Instant)>>;

#[derive(Clone, PartialEq, Eq)]
pub struct OidcRequest {
    pub url: String,
    pub query: Vec<(String, String)>,
    pub headers: Vec<(String, String)>,
    pub timeouts: OidcHttpTimeouts,
}

#[derive(Clone, PartialEq, Eq)]
pub struct OidcResponse {
    pub status: u16,
    pub body: Vec<u8>,
}

#[derive(Debug, Error)]
pub enum OidcHttpError {
    #[error("{message}")]
    Request { message: String },
}

pub trait OidcHttp: Send + Sync {
    fn get(&self, request: OidcRequest) -> Result<OidcResponse, OidcHttpError>;
}

#[derive(Debug, Error)]
pub enum AzureOidcError {
    #[error("{message}")]
    Provider { message: String },
}

pub trait AzureOidcTokens: Send + Sync {
    fn token(&self, scope: &str) -> Result<Option<SecretString>, AzureOidcError>;
}

pub trait OidcClock: Send + Sync {
    fn now(&self) -> Instant;

    fn unix_seconds(&self) -> i64;
}
