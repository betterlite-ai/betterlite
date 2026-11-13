use http::header::InvalidHeaderValue;
use indexmap::IndexMap;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum XaiSttError {
    #[error("xAI API key is required. Set XAI_API_KEY environment variable.")]
    MissingApiKey,
    #[error("the authorization header cannot carry the API key: {source}")]
    Authorization {
        #[source]
        source: InvalidHeaderValue,
    },
    #[error("the xAI transcription API answered with status {status}")]
    Status {
        status: u16,
        message: String,
        headers: IndexMap<String, String>,
    },
    #[error("the xAI transcription answer with status {status} is not the expected JSON: {source}")]
    Decode {
        status: u16,
        headers: IndexMap<String, String>,
        #[source]
        source: serde_json::Error,
    },
    #[error("the xAI transcription answer with status {status} exceeds the {limit}-byte limit")]
    TooLarge {
        status: u16,
        headers: IndexMap<String, String>,
        limit: usize,
    },
}

impl XaiSttError {
    // a missing key is the source's ValueError, so it carries no status; a decode failure keeps the reply's own
    #[must_use]
    pub const fn status_code(&self) -> Option<u16> {
        match self {
            Self::MissingApiKey | Self::Authorization { .. } => None,
            Self::Status { status, .. }
            | Self::Decode { status, .. }
            | Self::TooLarge { status, .. } => Some(*status),
        }
    }
}
