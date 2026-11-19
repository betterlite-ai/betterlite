use betterlite_error::error::ErrorClass;
use thiserror::Error;

use crate::response_input::error::ResponseInputError;
use crate::response_params::error::ResponseParamsError;
use crate::sail_common::error::SailError;

#[derive(Debug, Error)]
pub enum SailResponsesError {
    #[error(transparent)]
    Sail(#[from] SailError),
    #[error(transparent)]
    Params(#[from] ResponseParamsError),
    #[error(transparent)]
    Input(#[from] ResponseInputError),
    #[error("the mapped Responses parameters are not a JSON object: {source}")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
}

impl SailResponsesError {
    #[must_use]
    pub fn class(&self) -> Option<ErrorClass> {
        match self {
            Self::Sail(error) => Some(error.class()),
            Self::Params(_) | Self::Input(_) | Self::Encode { .. } => None,
        }
    }

    #[must_use]
    pub fn status_code(&self) -> u16 {
        match self {
            Self::Sail(error) => error.status_code(),
            Self::Params(error) => error.status_code(),
            Self::Input(error) => error.http_status().unwrap_or(500),
            Self::Encode { .. } => 500,
        }
    }
}
