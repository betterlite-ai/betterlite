use betterlite_config::loader::EnvironmentLookup;
use betterlite_openai::openai_base_config::Headers;
use secrecy::{ExposeSecret as _, SecretString};

use super::error::{VllmError, VllmSetupError};

pub const API_BASE_ENV: &str = "VLLM_API_BASE";
pub const API_KEY_HEADER: &str = "x-api-key";

// LiteLLM's VLLMModelInfo is a bag of helpers over the environment; get_api_key always returns None at the pin, so no key branch exists here, and get_base_model returns its argument, so callers pass the model through
pub struct VllmModelInfo<'environment> {
    pub environment: &'environment (dyn EnvironmentLookup + Sync),
}

impl VllmModelInfo<'_> {
    // a supplied or environment base falls through when it is empty, as the compatibility policy says
    pub fn get_api_base(&self, api_base: Option<&str>) -> Result<String, VllmSetupError> {
        let supplied = api_base.filter(|base| !base.is_empty()).map(str::to_owned);
        if let Some(base) = supplied {
            return Ok(base);
        }
        self.environment
            .lookup(API_BASE_ENV)
            .filter(|base| !base.is_empty())
            .ok_or(VllmSetupError::MissingApiBase)
    }

    pub fn get_models(&self, api_base: Option<&str>) -> Result<Vec<String>, VllmSetupError> {
        self.get_api_base(api_base)?;
        Err(VllmSetupError::MissingModelsCredentials)
    }

    pub fn validate_environment(&self, headers: &mut Headers, api_key: Option<&SecretString>) {
        if let Some(key) = api_key {
            headers.insert(String::from(API_KEY_HEADER), key.expose_secret().to_owned());
        }
    }

    #[must_use]
    pub fn get_error_class(&self, message: &str, status_code: u16, headers: &Headers) -> VllmError {
        VllmError::Api {
            status_code,
            message: message.to_owned(),
            headers: headers.clone(),
        }
    }
}
