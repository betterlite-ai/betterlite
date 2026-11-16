use betterlite_logging::analytics::python::FormatError;
use betterlite_openai::response_input::error::ResponseInputError;
use betterlite_openai::response_params::error::ResponseParamsError;
use betterlite_prompts::media::error::MediaError;
use betterlite_prompts::media_fetch::error::ImageFetchError;

use crate::aws_endpoints::error::AwsEndpointError;
use crate::bedrock_model_capabilities::CapabilityError;
use crate::bedrock_settings::{BedrockError, ErrorHeaders, ErrorInput, HeaderError};
use crate::signing::error::SigningError;

#[derive(Debug, thiserror::Error)]
pub enum ResponsesError {
    #[error(transparent)]
    Capability(#[from] CapabilityError),
    #[error(transparent)]
    Fetch(#[from] ImageFetchError),
    #[error(transparent)]
    Media(#[from] MediaError),
    #[error("no inlined data URL for the remote image {url}")]
    MissingInlined { url: String },
    #[error(transparent)]
    Params(#[from] ResponseParamsError),
    #[error(transparent)]
    Input(#[from] ResponseInputError),
    #[error("the Bedrock Runtime Responses body could not be encoded")]
    Encode(#[source] serde_json::Error),
    #[error(transparent)]
    Format(#[from] FormatError),
    #[error(transparent)]
    Region(#[from] AwsEndpointError),
    #[error(transparent)]
    Signing(#[from] SigningError),
}

// The OpenAI base builds a blank response, which drops x-amzn-RequestId, so this keeps the reply's headers
pub fn error_from_reply(
    status_code: u16,
    message: &str,
    headers: &[(String, String)],
) -> Result<BedrockError, HeaderError> {
    BedrockError::try_from(ErrorInput {
        status_code,
        message: message.to_owned(),
        request: None,
        body: None,
        status_code_is_synthesized: false,
        headers: Some(ErrorHeaders::Http(headers.to_vec())),
        response: None,
    })
}
