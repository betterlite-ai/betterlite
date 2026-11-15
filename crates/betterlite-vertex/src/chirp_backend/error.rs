use crate::chirp_stream::error::ChirpStreamError;
use crate::chirp_stream::target::AccessTokenError;

#[derive(Debug, thiserror::Error)]
pub enum ChirpBackendError {
    #[error("the Speech-to-Text session closed with code {code}")]
    Closed { code: u16, reason: String },
    #[error("the Speech-to-Text command was rejected")]
    Command(#[source] serde_json::Error),
    #[error("the Speech-to-Text event could not be encoded")]
    Encode(#[source] serde_json::Error),
    #[error("audio was sent before the Speech-to-Text stream was configured")]
    NotConfigured,
    #[error("the Speech-to-Text access token could not be resolved")]
    Token(#[from] AccessTokenError),
    #[error("the Speech-to-Text link queue is closed")]
    QueueClosed,
    #[error(transparent)]
    Stream(#[from] ChirpStreamError),
}
