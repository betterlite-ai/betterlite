use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use std::io::Error as IoError;
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CliAuthContract {
    pub contract_version: u8,
    pub issuer: String,
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    pub registration_endpoint: String,
    pub revocation_endpoint: String,
    pub resource: String,
    #[serde(rename = "code_challenge_methods_supported")]
    pub supported_methods: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct PkceCredential {
    pub access_token: SecretString,
    pub refresh_token: SecretString,
    pub expires_at: i64,
    pub client_id: String,
    pub token_endpoint: String,
    pub revocation_endpoint: String,
    pub resource: String,
    pub user_id: Option<String>,
    pub team_id: Option<String>,
}

#[derive(Debug, Error)]
pub enum PkceFailure {
    #[error("invalid base URL: {source}")]
    InvalidBaseUrl {
        #[source]
        source: url::ParseError,
    },
    #[error("discovery returned status {status}")]
    DiscoveryStatus { status: u16 },
    #[error("invalid discovery document: {source}")]
    DiscoveryDocument {
        #[source]
        source: serde_json::Error,
    },
    #[error("{0}")]
    Protocol(String),
    #[error("invalid registration request: {source}")]
    RegistrationRequest {
        #[source]
        source: serde_json::Error,
    },
    #[error("invalid registration response: {source}")]
    RegistrationResponse {
        #[source]
        source: serde_json::Error,
    },
    #[error("invalid token response: {source}")]
    TokenResponse {
        #[source]
        source: serde_json::Error,
    },
    #[error("callback timeout overflow")]
    CallbackTimeoutOverflow,
    #[error("timed out waiting for the browser sign-in to finish")]
    CallbackTimeout,
    #[error("loopback accept failed: {source}")]
    LoopbackAccept {
        #[source]
        source: IoError,
    },
    #[error("loopback write failed: {source}")]
    LoopbackWrite {
        #[source]
        source: IoError,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevocationUnavailable {
    pub reason: String,
}

#[derive(Debug, Clone)]
pub struct CallbackCode {
    pub code: SecretString,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallbackDenied {
    pub error: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone)]
pub enum CallbackOutcome {
    Code(CallbackCode),
    Denied(CallbackDenied),
}
