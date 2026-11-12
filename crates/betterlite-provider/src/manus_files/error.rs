use betterlite_http::error::HttpError;
use betterlite_secret_managers::secret_lookup::error::SecretLookupError;
use thiserror::Error;

use crate::api::file_contracts::config::Headers;
use crate::api::file_contracts::error::{BaseLlmException, FileContractError};

pub const MISSING_API_KEY_MESSAGE: &str =
    "Manus API key is required. Set MANUS_API_KEY environment variable or pass api_key parameter.";
pub const FILE_DATA_REQUIRED_MESSAGE: &str = "File data is required";
// LiteLLM lets these ValueErrors reach the caller without an upstream status, which the proxy turns into a 500.
const LOCAL_FAILURE_STATUS: u16 = 500;

#[derive(Debug, Error)]
pub enum ManusFilesError {
    #[error(
        "Manus API key is required. Set MANUS_API_KEY environment variable or pass api_key parameter."
    )]
    MissingApiKey,
    #[error("File data is required")]
    FileDataRequired,
    #[error("the {name} setting could not be read: {source}")]
    Environment {
        name: &'static str,
        #[source]
        source: SecretLookupError,
    },
    #[error("the {field} parameter must be text")]
    ParamNotText { field: &'static str },
    #[error("the Manus file id {file_id} cannot be written as a URL path segment: {source}")]
    FileId {
        file_id: String,
        #[source]
        source: HttpError,
    },
    #[error("Error parsing Manus file response: {source}")]
    ParseResponse {
        #[source]
        source: serde_json::Error,
    },
    #[error("the Manus list response is not a file list: {source}")]
    ListResponse {
        #[source]
        source: serde_json::Error,
    },
    #[error("the Manus delete response is not a file-deleted record: {source}")]
    DeleteResponse {
        #[source]
        source: serde_json::Error,
    },
}

impl From<ManusFilesError> for FileContractError {
    fn from(error: ManusFilesError) -> Self {
        Self::Upstream(BaseLlmException {
            status_code: LOCAL_FAILURE_STATUS,
            message: error.to_string(),
            headers: Headers::new(),
        })
    }
}
