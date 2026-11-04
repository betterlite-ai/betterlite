use betterlite_openai::error::{ErrorRequest, ErrorResponse};

#[derive(Debug, serde::Serialize, thiserror::Error)]
#[error("{message}")]
pub struct AwsAuthError {
    pub status: u16,
    pub message: String,
    pub request: ErrorRequest,
    pub response: ErrorResponse,
}

#[must_use]
pub fn aws_auth_error_projection(status: u16, message: String) -> AwsAuthError {
    let request = ErrorRequest {
        method: "POST".to_owned(),
        url: "https://us-west-2.console.aws.amazon.com/bedrock".to_owned(),
    };
    AwsAuthError {
        status,
        message,
        response: ErrorResponse {
            status_code: status,
            request: request.clone(),
        },
        request,
    }
}
