use super::{Headers, Object};
use serde::{Deserialize, Serialize};
use serde_json::Value;
pub struct ResponseForward<'input> {
    pub raw_text: &'input str,
    pub raw_headers: &'input Headers,
    pub decoded: Result<Value, serde_json::Error>,
    pub hidden_headers: &'input Headers,
}
pub fn forward_response<Sink, Projection>(
    sink: Sink,
    projection: Projection,
    input: ResponseForward<'_>,
) -> Result<Value, BaseConfigError>
where
    Sink: FnOnce(&str, &Headers) -> Result<(), ForwardingFailure>,
    Projection: FnOnce(Value, &Headers, &Headers) -> Result<Value, ForwardingFailure>,
{
    sink(input.raw_text, input.raw_headers).map_err(BaseConfigError::Sink)?;
    projection(
        input.decoded.map_err(BaseConfigError::Decode)?,
        input.raw_headers,
        input.hidden_headers,
    )
    .map_err(BaseConfigError::Projection)
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
    Sink(#[source] ForwardingFailure),
    #[error("response projection failed: {0}")]
    Projection(#[source] ForwardingFailure),
}
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("injected forwarding failure: {message}")]
pub struct ForwardingFailure {
    pub message: String,
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
