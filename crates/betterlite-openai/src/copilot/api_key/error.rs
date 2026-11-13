use std::io;

use thiserror::Error;

use crate::copilot::device_auth::error::DeviceAuthError;

#[derive(Debug, Error)]
pub enum ApiKeyError {
    #[error("the Copilot access token could not be obtained")]
    AccessToken(#[source] DeviceAuthError),
    #[error("no Copilot API key after three refresh attempts")]
    Refresh,
    #[error("the Copilot API key record could not be saved")]
    Save(#[source] io::Error),
    #[error("the Copilot API key record could not be encoded")]
    Encode(#[source] serde_json::Error),
    #[error("the Copilot API key record has no token")]
    MissingToken,
}

impl ApiKeyError {
    #[must_use]
    pub fn status(&self) -> u16 {
        match self {
            Self::AccessToken(source) => source.status().unwrap_or(401),
            Self::Refresh | Self::MissingToken => 401,
            Self::Save(_) | Self::Encode(_) => 500,
        }
    }
}
