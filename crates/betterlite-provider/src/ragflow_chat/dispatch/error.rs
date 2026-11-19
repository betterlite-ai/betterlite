use thiserror::Error;

use super::handler::HandlerError;
use crate::ragflow_chat::error::RagflowError;

#[derive(Debug, Error)]
pub enum DispatchError {
    #[error(transparent)]
    Provider(#[from] RagflowError),
    #[error("the post_call input could not be written as JSON: {source}")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
    #[error(transparent)]
    Handler(#[from] HandlerError),
}
