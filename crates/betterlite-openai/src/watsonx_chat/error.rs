use thiserror::Error;

use crate::watsonx_auth::error::WatsonxError;

#[derive(Debug, Error)]
pub enum WatsonxChatError {
    #[error(transparent)]
    Auth(#[from] WatsonxError),
    #[error("{field} has the wrong type")]
    InvalidField { field: &'static str },
    #[error("the tools could not be written as JSON")]
    Json(#[source] serde_json::Error),
}
