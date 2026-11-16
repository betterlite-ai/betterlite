use crate::aws_endpoints::error::AwsEndpointError;
use crate::signing::error::SigningError;
use betterlite_context::chat_config_selection::selection::ChatConfigError;
use betterlite_openai::error::{ProviderError, ProviderErrorParts};
use betterlite_openai::openai_base_config::Headers;
use betterlite_openai::passthrough::transformation::types::PassthroughError;
use std::fmt;
use thiserror::Error;
#[derive(Error)]
#[error("the provider request failed")]
pub struct ProviderFailure(pub Box<ProviderError>);
impl fmt::Debug for ProviderFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ProviderFailure([REDACTED])")
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
    Reply(#[source] ProviderFailure),
    #[error("the provider request failed")]
    Provider(#[source] ProviderFailure),
}
impl From<ProviderError> for PassthroughFailure {
    fn from(error: ProviderError) -> Self {
        Self::Provider(ProviderFailure(Box::new(error)))
    }
}
#[must_use]
pub fn bedrock_provider_error(message: &str, status_code: u16, headers: &Headers) -> ProviderError {
    ProviderError::from(ProviderErrorParts {
        status_code,
        message: message.to_owned(),
        headers: (!headers.is_empty()).then(|| {
            headers
                .iter()
                .map(|(name, value)| (name.clone(), value.clone()))
                .collect()
        }),
        request: None,
        response: None,
        body: None,
        status_code_is_synthesized: false,
    })
}
