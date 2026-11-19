use thiserror::Error;

use crate::aws_endpoints::error::AwsEndpointError;
use crate::mantle_inputs::error::MantleInputError;
use crate::messages::config::ConstructorError;
use crate::signing::error::SigningError;

// BedrockMantleAuthMixin.sign_request's ValueError text, byte for byte.
pub const AUTH_FAILURE_MESSAGE: &str = "Bedrock Mantle auth failed: no Bearer token and no usable AWS credentials. Set BEDROCK_MANTLE_API_KEY (or AWS_BEARER_TOKEN_BEDROCK) or pass api_key for Bearer auth, or provide AWS credentials (IAM role / access key / profile / web identity) for SigV4.";

#[derive(Debug, Error)]
pub enum MantleSettingsError {
    #[error("{AUTH_FAILURE_MESSAGE}")]
    Auth {
        #[source]
        source: Box<SigningError>,
    },
    #[error(transparent)]
    Region(#[from] MantleInputError),
    #[error(transparent)]
    Signing(#[from] SigningError),
    #[error(transparent)]
    Constructor(#[from] ConstructorError),
    #[error("the field {field} is not a string")]
    Field { field: &'static str },
    #[error("the config attributes are not an object")]
    Attributes,
    #[error("a request parameter could not be encoded")]
    Encode(#[source] serde_json::Error),
    #[error(transparent)]
    Endpoint(#[from] AwsEndpointError),
}
