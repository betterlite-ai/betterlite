use thiserror::Error;

use crate::api::response_chat_request::error::RequestError;
use crate::api::response_id_traversal::response_id::ResponseRewriteError;
use crate::api::responses_parameters::error::ResponsesParametersError;

#[derive(Debug, Error)]
pub enum WsNativeError {
    #[error("'{kind}' object is not a mapping")]
    TypeError { kind: &'static str },
    #[error("'{kind}' object has no attribute 'get'")]
    AttributeError { kind: &'static str },
    #[error("{message}")]
    Validation { message: String },
    #[error(transparent)]
    Parameters(#[from] ResponsesParametersError),
    #[error(transparent)]
    OptionalParams(#[from] RequestError),
    #[error("the frame could not be written as JSON: {0}")]
    Encode(#[from] serde_json::Error),
    #[error(transparent)]
    Rewrite(#[from] ResponseRewriteError),
}
