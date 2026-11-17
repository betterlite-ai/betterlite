use thiserror::Error;

use crate::response_input::error::ResponseInputError;

#[derive(Debug, Error)]
pub enum FireworksResponsesError {
    #[error(transparent)]
    Input(#[from] ResponseInputError),
    #[error("the delete response result could not be encoded")]
    Delete {
        #[source]
        source: serde_json::Error,
    },
}

impl FireworksResponsesError {
    #[must_use]
    pub const fn status_code(&self) -> Option<u16> {
        match self {
            Self::Input(error) => error.http_status(),
            Self::Delete { .. } => None,
        }
    }
}
