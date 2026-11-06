use thiserror::Error;

use crate::generic_prompt_api::variables::VariablesError;

#[derive(Debug, Error)]
pub enum PromptFetchError<TransportError> {
    #[error("prompt_id or prompt_spec is required")]
    MissingPromptSource,
    #[error("query parameter '{key}' must be a string, number, bool, null or a list of those")]
    QueryValue { key: String },
    #[error("Failed to fetch prompt '{prompt_id}' from API: {source}")]
    Transport {
        prompt_id: String,
        #[source]
        source: TransportError,
    },
    #[error("Failed to fetch prompt '{prompt_id}' from API: the server answered {status}")]
    Status { prompt_id: String, status: u16 },
    #[error("Failed to parse prompt response for '{prompt_id}': {source}")]
    Parse {
        prompt_id: String,
        #[source]
        source: serde_json::Error,
    },
}

#[derive(Debug, Error)]
pub enum PromptCompileError<TransportError> {
    #[error("Error compiling prompt '{prompt_id}': {source}")]
    Fetch {
        prompt_id: String,
        #[source]
        source: PromptFetchError<TransportError>,
    },
    #[error(transparent)]
    Variables(#[from] VariablesError),
}
