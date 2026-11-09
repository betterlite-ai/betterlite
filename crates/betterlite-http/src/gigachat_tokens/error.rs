use http::StatusCode;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum GigachatAuthError {
    #[error("GigaChat credentials are not set: fill in GIGACHAT_CREDENTIALS or GIGACHAT_API_KEY")]
    MissingCredentials,
    #[error("the GigaChat OAuth server answered with status {status}")]
    Rejected { status: StatusCode, body: String },
    #[error("the GigaChat OAuth request did not complete")]
    Transport {
        #[source]
        source: reqwest::Error,
    },
    #[error("the GigaChat token reply is not usable")]
    InvalidReply,
}

impl GigachatAuthError {
    #[must_use]
    pub fn status_code(&self) -> StatusCode {
        match self {
            Self::MissingCredentials => StatusCode::UNAUTHORIZED,
            Self::Rejected { status, .. } => *status,
            Self::Transport { source } if source.is_timeout() => StatusCode::REQUEST_TIMEOUT,
            Self::Transport { .. } | Self::InvalidReply => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}
