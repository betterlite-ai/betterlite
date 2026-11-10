use betterlite_http::proxy::transport_error::TransportError;
use thiserror::Error;

use crate::openai_base_config::forwarding::BaseConfigError;

#[derive(Debug, Error)]
pub enum ModelInfoError {
    #[error("the models endpoint cannot be built: {0}")]
    Endpoint(#[source] BaseConfigError),
    #[error("the models URL is not valid: {0}")]
    Url(#[source] url::ParseError),
    #[error("a request header is not valid: {0}")]
    Header(#[source] http::Error),
    #[error("the models request failed: {0}")]
    Transport(#[source] TransportError),
    #[error("the models endpoint answered with status {status}")]
    Status { status: u16 },
    #[error("the model list is not valid: {0}")]
    Decode(#[source] serde_json::Error),
    #[error("the model list is not a JSON object")]
    NotAnObject,
}
