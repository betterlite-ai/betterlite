pub mod reply;
pub mod request;
pub mod tags;
pub mod titan;

mod tests;

use std::string::FromUtf8Error;

use thiserror::Error;

use crate::s3_access::S3AccessError;
use crate::signing::error::SigningError;

#[derive(Debug, Error)]
pub enum BatchCreateError {
    #[error("input_file_id is required for Bedrock batch creation")]
    InputFileIdRequired,
    #[error(
        "AWS IAM role ARN is required for Bedrock batch jobs. Set 'aws_batch_role_arn' in litellm_params or AWS_BATCH_ROLE_ARN env var"
    )]
    RoleArnRequired,
    #[error("Could not determine Bedrock model ID. Please pass `model` in your request body.")]
    ModelRequired,
    #[error(
        "Invalid 'bedrock_tags' value. Expected a list of {{'key': <str>, 'value': <str>}} dicts, e.g. [{{'key': 'team', 'value': 'genai'}}]. Got: {raw}"
    )]
    InvalidTags { raw: String },
    #[error("{field}: Input has the wrong type")]
    Field {
        field: &'static str,
        #[source]
        source: serde_json::Error,
    },
    #[error("the Bedrock batch request cannot be serialized")]
    RequestJson(#[source] serde_json::Error),
    #[error("Failed to parse Bedrock batch response: {source}")]
    ResponseParse {
        #[source]
        source: serde_json::Error,
    },
    #[error("the signed request body is not UTF-8")]
    SignedBody(#[source] FromUtf8Error),
    #[error(transparent)]
    S3(#[from] S3AccessError),
    #[error(transparent)]
    Signing(#[from] SigningError),
}
