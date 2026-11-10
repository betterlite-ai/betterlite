use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeRequest {
    pub method: String,
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeResponse {
    pub status_code: u16,
    pub request: NativeRequest,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{message}")]
pub struct NativeError {
    pub status_code: u16,
    pub message: String,
    pub request: NativeRequest,
    pub response: NativeResponse,
}

impl NativeError {
    #[must_use]
    pub fn new(
        status_code: u16,
        message: String,
        supplied_request: Option<NativeRequest>,
        supplied_response: Option<NativeResponse>,
    ) -> Self {
        let request = supplied_request.unwrap_or_else(|| NativeRequest {
            method: "POST".to_owned(),
            url: "https://docs.codestral.com/user-guide/inference/rest_api".to_owned(),
        });
        let response = supplied_response.unwrap_or_else(|| NativeResponse {
            status_code,
            request: request.clone(),
        });
        Self {
            status_code,
            message,
            request,
            response,
        }
    }
}

#[derive(Debug, Error)]
pub enum TextCompletionCodestralError {
    #[error("{0}")]
    Native(Box<NativeError>),
    #[error("the codestral record is {size} bytes, over the {limit} byte limit")]
    TooLarge { size: usize, limit: usize },
    #[error("the codestral stream chunk is not a stream chunk object")]
    Stream(#[source] serde_json::Error),
}
