use super::{Headers, Object};
use serde::{Deserialize, Serialize};
use serde_json::Value;
pub type PostCallSink = fn(&str, &Headers) -> Result<(), BaseConfigError>;
pub type ChatResponseProjection = fn(Value, &Headers, &Headers) -> Result<Value, BaseConfigError>;
pub type StreamProjection = fn(Value) -> Result<Value, BaseConfigError>;
pub struct ResponseForward<'input> {
    pub raw_text: &'input str,
    pub raw_headers: &'input Headers,
    pub decoded: Result<Value, serde_json::Error>,
    pub hidden_headers: &'input Headers,
}
pub fn forward_response(
    sink: PostCallSink,
    projection: ChatResponseProjection,
    input: ResponseForward<'_>,
) -> Result<Value, BaseConfigError> {
    sink(input.raw_text, input.raw_headers)?;
    projection(
        input.decoded.map_err(BaseConfigError::Decode)?,
        input.raw_headers,
        input.hidden_headers,
    )
}
pub fn forward_stream(
    projection: StreamProjection,
    response: Value,
) -> Result<Value, BaseConfigError> {
    let projected = projection(response);
    projected
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OpenAiErrorRecord {
    pub status_code: u16,
    pub message: String,
    pub headers: Option<Headers>,
    pub request: ErrorRequest,
    pub response: ErrorResponse,
    pub body: Option<Object>,
}
impl OpenAiErrorRecord {
    #[must_use]
    pub fn from_parts(parts: OpenAiErrorParts) -> Self {
        let default_request = parts.request.unwrap_or_else(|| ErrorRequest {
            method: String::from("POST"),
            url: String::from("https://api.openai.com/v1"),
        });
        let default_response = parts.response.unwrap_or_else(|| ErrorResponse {
            status_code: parts.status_code,
            request: default_request.clone(),
        });
        Self {
            status_code: parts.status_code,
            message: parts.message,
            headers: parts.headers,
            request: default_request,
            response: default_response,
            body: parts.body,
        }
    }
}
pub struct OpenAiErrorParts {
    pub status_code: u16,
    pub message: String,
    pub headers: Option<Headers>,
    pub request: Option<ErrorRequest>,
    pub response: Option<ErrorResponse>,
    pub body: Option<Object>,
}

#[derive(Debug, thiserror::Error)]
pub enum BaseConfigError {
    #[error("invalid api base URL: {0}")]
    InvalidUrl(#[source] url::ParseError),
    #[error("response JSON could not be decoded: {0}")]
    Decode(#[source] serde_json::Error),
    #[error("post-call sink failed: {0}")]
    Sink(String),
    #[error("response projection failed: {0}")]
    Projection(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorRequest {
    pub method: String,
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorResponse {
    pub status_code: u16,
    pub request: ErrorRequest,
}
