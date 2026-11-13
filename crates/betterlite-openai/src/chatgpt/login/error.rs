use super::codec::DecodeError;
use super::http::HttpFailure;

#[derive(Debug, thiserror::Error)]
pub enum Failure {
    #[error("the server answered with status {0}")]
    Status(u16),
    #[error("the request could not be completed")]
    Transport(#[from] HttpFailure),
    #[error("the reply could not be used")]
    Decode(#[from] DecodeError),
    #[error("the request body could not be written")]
    Encode(#[from] serde_json::Error),
    #[error("timed out waiting for device authorization")]
    Expired,
}

#[derive(Debug, thiserror::Error)]
pub enum LoginError {
    #[error("failed to request device code")]
    DeviceCode(#[source] Failure),
    #[error("failed to obtain an access token")]
    AccessToken(#[source] Failure),
    #[error("refresh token failed")]
    Refresh(#[source] Failure),
}

impl LoginError {
    // the upstream status when the server refused, 408 when polling ran out, 400 for transport and parse failures
    #[must_use]
    pub const fn status(&self) -> u16 {
        let (Self::DeviceCode(cause) | Self::AccessToken(cause) | Self::Refresh(cause)) = self;
        match cause {
            Failure::Status(status) => *status,
            Failure::Expired => 408,
            Failure::Transport(_) | Failure::Decode(_) | Failure::Encode(_) => 400,
        }
    }
}
