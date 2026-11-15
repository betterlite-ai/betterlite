use std::collections::BTreeMap;

use betterlite_openai::error::{ProviderError, ProviderErrorParts};
use betterlite_openai::openai_base_config::Headers;
use betterlite_prompts::error::FactoryError;
use thiserror::Error;

// the local torch branch cannot run in Rust, so its paths answer 400 not supported (docs/deviations/B8153.yaml)
pub const ENGINE_STATUS: u16 = 400;
pub const ENGINE_MISSING_MESSAGE: &str = "Importing torch, transformers, petals failed\nTry pip installing petals \npip install git+https://github.com/bigscience-workshop/petals";

#[derive(Debug, Error)]
pub enum PetalsSetupError {
    #[error("{message}")]
    Engine {
        status_code: u16,
        message: &'static str,
    },
    #[error("the custom prompt for {model} has no {field} entry")]
    CustomPromptField { model: String, field: &'static str },
    #[error(transparent)]
    Prompt(#[from] FactoryError),
    #[error("the prompt factory returned an ollama prompt for a petals request")]
    OllamaPrompt,
    #[error("the petals environment is invalid: {source}")]
    Environment {
        #[source]
        source: betterlite_env::EnvError,
    },
}

impl PetalsSetupError {
    #[must_use]
    pub const fn status(&self) -> Option<u16> {
        match self {
            Self::Engine { status_code, .. } => Some(*status_code),
            Self::CustomPromptField { .. }
            | Self::Prompt(_)
            | Self::OllamaPrompt
            | Self::Environment { .. } => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{message}")]
pub struct PetalsError {
    pub status_code: u16,
    pub message: String,
    pub headers: BTreeMap<String, String>,
}

impl From<PetalsError> for ProviderError {
    fn from(error: PetalsError) -> Self {
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
pub fn get_error_class(message: &str, status_code: u16, headers: &Headers) -> PetalsError {
    PetalsError {
        status_code,
        message: message.to_owned(),
        headers: headers.clone(),
    }
}
