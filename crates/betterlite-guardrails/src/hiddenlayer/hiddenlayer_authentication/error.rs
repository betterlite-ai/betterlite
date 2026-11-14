use std::time::TryFromFloatSecsError;

use betterlite_http::proxy::limits::ReadError;
use thiserror::Error;
use tokio::task::JoinError;

use super::token::JwtError;

#[derive(Debug, Error)]
pub enum HiddenlayerError {
    #[error("`api_id` cannot be None when using the SaaS version of HiddenLayer.")]
    MissingApiId,
    #[error("`api_key` cannot be None when using the SaaS version of HiddenLayer.")]
    MissingApiKey,
    #[error("the HiddenLayer auth timeout is not a usable number of seconds")]
    InvalidTimeout {
        #[source]
        source: TryFromFloatSecsError,
    },
    #[error(transparent)]
    Jwt(#[from] JwtError),
    #[error("the HiddenLayer detection request could not be sent")]
    Transport {
        #[source]
        source: reqwest::Error,
    },
    #[error("the HiddenLayer detection response body could not be read")]
    Body {
        #[source]
        source: ReadError,
    },
    #[error("the HiddenLayer detection endpoint returned {status}: {body}")]
    Status { status: u16, body: String },
    #[error("the HiddenLayer token refresh task could not be joined")]
    RefreshTask {
        #[source]
        source: JoinError,
    },
}
