use std::collections::BTreeMap;

use betterlite_openai::error::{ProviderError, ProviderErrorParts};
use betterlite_prompts::error::FactoryError;
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{message}")]
pub struct PredibaseError {
    pub status_code: u16,
    pub message: String,
    pub headers: BTreeMap<String, String>,
}

impl From<PredibaseError> for ProviderError {
    fn from(error: PredibaseError) -> Self {
        let headers: Vec<(String, String)> = error.headers.into_iter().collect();
        Self::from(ProviderErrorParts {
            status_code: error.status_code,
            message: error.message,
            headers: (!headers.is_empty()).then_some(headers),
            request: None,
            response: None,
            body: None,
            status_code_is_synthesized: false,
        })
    }
}

#[must_use]
pub fn get_error_class(
    message: &str,
    status_code: u16,
    headers: &BTreeMap<String, String>,
) -> PredibaseError {
    PredibaseError {
        status_code,
        message: message.to_owned(),
        headers: headers.clone(),
    }
}

#[derive(Debug, Error)]
pub enum PredibaseSetupError {
    #[error(
        "Missing Predibase Tenant ID - Required for making the request. Set dynamically (e.g. `completion(..tenant_id=<MY-ID>)`) or in env - `PREDIBASE_TENANT_ID`."
    )]
    MissingTenantId,
    #[error(
        "Missing Predibase API Key - A call is being made to predibase but no key is set either in the environment variables or via params"
    )]
    MissingApiKey,
    #[error("the custom prompt for {model} has no {field} entry")]
    CustomPromptField { model: String, field: &'static str },
    #[error(transparent)]
    Prompt(#[from] FactoryError),
    #[error("the prompt factory returned an ollama prompt for a predibase request")]
    OllamaPrompt,
}
