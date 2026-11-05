use betterlite_openai::error::{ErrorRequest, ErrorResponse};

pub const AWS_CONSOLE_URL: &str = "https://us-west-2.console.aws.amazon.com/bedrock";

#[derive(Debug, serde::Serialize, thiserror::Error)]
#[error("{message}")]
pub struct AwsAuthError {
    pub status: u16,
    pub message: String,
    pub request: ErrorRequest,
    pub response: ErrorResponse,
}
