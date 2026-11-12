use betterlite_models::decisions::Json;
use betterlite_openai::api::image_contracts::records::{ImageHeaders, ImageParams};
use betterlite_openai::error::{ProviderError, ProviderErrorParts};
use secrecy::SecretString;

#[derive(Debug, Clone, Copy)]
pub struct ImageUrlRequest<'request> {
    pub api_base: Option<&'request str>,
    pub api_key: Option<&'request SecretString>,
    pub model: &'request str,
    pub optional_params: &'request ImageParams,
    pub litellm_params: &'request ImageParams,
    pub stream: Option<bool>,
}

#[derive(Debug, Clone, Copy)]
pub struct ImageEnvironment<'request> {
    pub headers: &'request ImageHeaders,
    pub model: &'request str,
    pub messages: &'request [Json],
    pub optional_params: &'request ImageParams,
    pub litellm_params: &'request ImageParams,
    pub api_key: Option<&'request SecretString>,
    pub api_base: Option<&'request str>,
}

// B2202's BaseLLMException projection: the three supplied facts, no error type, param or code
#[must_use]
pub fn base_exception(
    error_message: &str,
    status_code: u16,
    headers: &ImageHeaders,
) -> ProviderError {
    let parts = ProviderErrorParts {
        status_code,
        message: error_message.to_owned(),
        headers: Some(
            headers
                .iter()
                .map(|(name, value)| (name.clone(), value.clone()))
                .collect(),
        ),
        request: None,
        response: None,
        body: None,
        status_code_is_synthesized: false,
    };
    ProviderError::from(parts)
}
