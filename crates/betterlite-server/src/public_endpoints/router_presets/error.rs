use betterlite_http::proxy::transport_error::TransportError;
use betterlite_router::complexity_scoring::error::ScoringError;
use betterlite_router::fuse_presets::error::FusePresetsError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum RouterPresetsError {
    #[error("the auto-router preset catalog does not match its wire shape")]
    Decode(#[source] Box<serde_json::Error>),
    #[error("the auto-router preset catalog is {size} bytes, over the {limit} byte limit")]
    Oversize { size: usize, limit: usize },
    #[error("the remote auto-router preset catalog is empty")]
    Empty,
    #[error("the remote auto-router preset source answered with status {status}")]
    Status { status: u16 },
    #[error("the auto-router preset source is not a URL")]
    Url(#[source] Box<url::ParseError>),
    #[error("the auto-router preset request failed")]
    Request(#[source] TransportError),
    #[error(transparent)]
    Scorer(#[from] ScoringError),
    #[error(transparent)]
    Fuse(#[from] FusePresetsError),
}
