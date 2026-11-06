use std::io;

use thiserror::Error;

use super::decode::ReplyError;
use super::transport::TransportFailure;

#[derive(Debug, Error)]
pub enum DeviceAuthError {
    #[error("the Copilot token directory could not be created")]
    TokenDirectory(#[source] io::Error),
    #[error("the GitHub device code request failed")]
    DeviceCode(#[source] DeviceCodeFailure),
    #[error("the GitHub access token request failed")]
    AccessToken(#[source] AccessTokenFailure),
    #[error("no Copilot access token after three login attempts")]
    Exhausted,
    #[error("the Copilot API key expired")]
    ApiKeyExpired,
    #[error("the Copilot API key refresh failed")]
    RefreshApiKey,
    #[error("the Copilot API key request failed with status {status}")]
    ApiKey { status: u16 },
}

impl DeviceAuthError {
    #[must_use]
    pub const fn status(&self) -> Option<u16> {
        match self {
            Self::TokenDirectory(_) => None,
            Self::DeviceCode(_) | Self::AccessToken(_) => Some(400),
            Self::Exhausted | Self::ApiKeyExpired | Self::RefreshApiKey => Some(401),
            Self::ApiKey { status } => Some(*status),
        }
    }
}

#[derive(Debug, Error)]
pub enum DeviceCodeFailure {
    #[error("the device code request could not be sent")]
    Transport(#[source] TransportFailure),
    #[error("GitHub answered the device code request with status {status}")]
    Status { status: u16 },
    #[error("the device code reply could not be decoded")]
    Reply(#[source] ReplyError),
    #[error("the device code reply has no {name}")]
    MissingField { name: &'static str },
    #[error("the device code request could not be encoded")]
    Encode(#[source] serde_json::Error),
}

#[derive(Debug, Error)]
pub enum AccessTokenFailure {
    #[error("the access token request could not be sent")]
    Transport(#[source] TransportFailure),
    #[error("GitHub answered the access token request with status {status}")]
    Status { status: u16 },
    #[error("the access token reply could not be decoded")]
    Reply(#[source] ReplyError),
    #[error("the device authorization waited through every poll")]
    Timeout,
    #[error("the access token request could not be encoded")]
    Encode(#[source] serde_json::Error),
}
