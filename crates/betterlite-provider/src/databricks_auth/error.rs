use std::collections::BTreeMap;

use betterlite_openai::error::{ProviderError, ProviderErrorParts};
use thiserror::Error;

pub const MISSING_API_KEY: &str = "Missing API Key - A call is being made to LLM Provider but no key is set either in the environment variables ({LLM_PROVIDER}_API_KEY) or via params";
pub const MISSING_API_BASE: &str = "Missing API Base - A call is being made to LLM Provider but no api base is set either in the environment variables ({LLM_PROVIDER}_API_KEY) or via params";
pub const SDK_CREDENTIALS: &str = "If the Databricks base URL and API key are not set, the databricks-sdk Python library must be installed. Please install the databricks-sdk, set {LLM_PROVIDER}_API_BASE and {LLM_PROVIDER}_API_KEY environment variables, or provide the base URL and API key as arguments.";
pub const SDK_API_BASE: &str = "Either set the DATABRICKS_API_BASE and DATABRICKS_API_KEY environment variables, or install the databricks-sdk Python library.";

// LiteLLM's DatabricksException is BaseLLMException: a status, a message and optional upstream headers
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{message}")]
pub struct DatabricksError {
    pub status_code: u16,
    pub message: String,
    pub headers: Option<BTreeMap<String, String>>,
}

impl DatabricksError {
    #[must_use]
    pub(crate) const fn new(status_code: u16, message: String) -> Self {
        Self {
            status_code,
            message,
            headers: None,
        }
    }
}

impl From<DatabricksError> for ProviderError {
    fn from(error: DatabricksError) -> Self {
        let headers: Option<Vec<(String, String)>> = error
            .headers
            .map(|pairs| pairs.into_iter().collect::<Vec<_>>())
            .filter(|pairs| !pairs.is_empty());
        Self::from(ProviderErrorParts {
            status_code: error.status_code,
            message: error.message,
            headers,
            request: None,
            response: None,
            body: None,
            status_code_is_synthesized: false,
        })
    }
}
