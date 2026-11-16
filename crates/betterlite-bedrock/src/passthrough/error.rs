use crate::aws_endpoints::error::AwsEndpointError;
use crate::bedrock_settings::{BedrockError, ErrorHeaders, ErrorInput};
use crate::signing::error::SigningError;
use betterlite_context::chat_config_selection::selection::ChatConfigError;
use betterlite_openai::error::{ProviderError, ProviderErrorParts};
use betterlite_openai::openai_base_config::Headers;
use betterlite_openai::passthrough::transformation::types::PassthroughError;
use std::fmt;
use thiserror::Error;
pub struct ProviderFailure(pub ProviderError);
impl fmt::Debug for ProviderFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ProviderFailure([REDACTED])")
    }
}
impl fmt::Display for ProviderFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("the provider request failed")
    }
}
#[derive(Debug, Error)]
pub enum PassthroughFailure {
    #[error(transparent)]
    Url(#[from] PassthroughError),
    #[error(transparent)]
    Endpoint(#[from] AwsEndpointError),
    #[error(transparent)]
    Signing(#[from] SigningError),
    #[error(transparent)]
    Selection(#[from] ChatConfigError),
    #[error("the field {field} is not a string")]
    Field { field: &'static str },
    #[error("the request data could not be encoded")]
    Body(#[source] serde_json::Error),
    #[error("the provider {provider} is not a known provider")]
    UnknownProvider { provider: String },
    #[error("No provider config found for model: {model}")]
    NoProviderConfig { model: String },
    #[error("the native reply transform failed")]
    Reply(ProviderFailure),
    #[error("the provider request failed")]
    Provider(ProviderFailure),
}
impl From<ProviderError> for PassthroughFailure {
    fn from(error: ProviderError) -> Self {
        Self::Provider(ProviderFailure(error))
    }
}
pub fn bedrock_provider_error(message: &str, status_code: u16, headers: &Headers) -> ProviderError {
    let input = ErrorInput {
        status_code,
        message: message.to_owned(),
        request: None,
        body: None,
        status_code_is_synthesized: false,
        headers: (!headers.is_empty()).then(|| {
            ErrorHeaders::Http(
                headers
                    .iter()
                    .map(|(name, value)| (name.clone(), value.clone()))
                    .collect(),
            )
        }),
        response: None,
    };
    match BedrockError::try_from(input) {
        Ok(error) => error.error,
        Err(_source) => ProviderError::from(ProviderErrorParts {
            status_code,
            message: message.to_owned(),
            headers: None,
            request: None,
            response: None,
            body: None,
            status_code_is_synthesized: false,
        }),
    }
}
