use std::any::type_name;
use std::error::Error;
use std::sync::Arc;

use betterlite_prompts::arguments::json::{Json, Object};
use http::StatusCode;
use rmcp::ServiceError;
use serde::{Deserialize, Serialize};

use crate::client::cleanup::http_client::ResponseHead;
use crate::client::http_transport::error::McpHttpError;
use crate::client::sdk_transport::error::SdkError;
use crate::client::sessions::error::SessionError;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    Text(TextContent),
    Image(BinaryContent),
    Audio(BinaryContent),
    ResourceLink(ResourceLink),
    Resource(EmbeddedResource),
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TextContent {
    pub text: String,
    #[serde(flatten)]
    pub extra: Object,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BinaryContent {
    pub data: String,
    #[serde(rename = "mimeType")]
    pub mime_type: String,
    #[serde(flatten)]
    pub extra: Object,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResourceLink {
    pub uri: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(rename = "mimeType", default, skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    #[serde(flatten)]
    pub extra: Object,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct EmbeddedResource {
    pub resource: ResourceContents,
    #[serde(flatten)]
    pub extra: Object,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResourceContents {
    pub uri: String,
    #[serde(rename = "mimeType", default, skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blob: Option<String>,
    #[serde(flatten)]
    pub extra: Object,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CallToolResult {
    pub content: Vec<ContentBlock>,
    #[serde(rename = "isError", default)]
    pub is_error: bool,
    #[serde(
        rename = "structuredContent",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub structured_content: Option<Json>,
    #[serde(rename = "_meta", default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<Object>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct InputRequiredResult {
    #[serde(
        rename = "inputRequests",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub input_requests: Option<Object>,
    #[serde(
        rename = "requestState",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub request_state: Option<String>,
    #[serde(rename = "resultType")]
    pub result_type: String,
    #[serde(rename = "_meta", default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<Object>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(untagged)]
pub enum ToolCallResult {
    Complete(CallToolResult),
    InputRequired(InputRequiredResult),
}

// error_text_result is the unresolved helper LiteLLM delegates to; it builds the one text block a swallowed failure becomes
#[must_use]
pub fn error_text_result(error_type: &str, message: &str) -> ToolCallResult {
    ToolCallResult::Complete(CallToolResult {
        content: vec![ContentBlock::Text(TextContent {
            text: format!("{error_type}: {message}"),
            extra: Object::new(),
        })],
        is_error: true,
        structured_content: None,
        meta: None,
    })
}

fn short_type_name<Value>() -> &'static str
where
    Value: ?Sized,
{
    let name = type_name::<Value>();
    let tail = name.rsplit("::").next().unwrap_or(name);
    tail.split('<').next().unwrap_or(tail)
}

pub(crate) struct Failure {
    pub(crate) error_type: &'static str,
    pub(crate) message: String,
}

// LiteLLM reports type(exc).__name__; the Rust analogue is the source type's last path segment
pub(crate) fn describe_failure<Source>(error: &SessionError<Source>) -> Failure
where
    Source: Error + Send + Sync + 'static,
{
    match error {
        SessionError::Runtime(source) => {
            if let Some(failure) = status_failure(source) {
                return failure;
            }
            Failure {
                error_type: short_type_name::<Source>(),
                message: source.to_string(),
            }
        }
        SessionError::Stream(failure) => Failure {
            error_type: short_type_name::<Source>(),
            message: failure.source_error().to_string(),
        },
        SessionError::Protocol(failure) => Failure {
            error_type: "MCPError",
            message: failure.message.clone(),
        },
        SessionError::ReadTimeout(timeout) => Failure {
            error_type: "TimeoutError",
            message: timeout.message.clone(),
        },
        SessionError::CleanupTask(cause) => Failure {
            error_type: "JoinError",
            message: cause.to_string(),
        },
        SessionError::Cancelled | SessionError::CleanupTimedOut => Failure {
            error_type: "CancelledError",
            message: error.to_string(),
        },
        SessionError::Configuration(_) => Failure {
            error_type: "ValueError",
            message: error.to_string(),
        },
    }
}

// httpx2's HTTPStatusError text, so a swallowed failure and a raised one read the same (client.py L249-L251)
fn status_failure<Source>(source: &Arc<Source>) -> Option<Failure>
where
    Source: Error + Send + Sync + 'static,
{
    let head = status_head(source.as_ref())?;
    let request_url = head
        .capture
        .as_ref()
        .map(|record| record.exchange.url.as_str());
    Some(Failure {
        error_type: "HTTPStatusError",
        message: http_status_message(head.status, request_url),
    })
}

#[must_use]
pub fn http_status_message(status: u16, url: Option<&str>) -> String {
    let reason = StatusCode::from_u16(status)
        .ok()
        .as_ref()
        .and_then(StatusCode::canonical_reason)
        .unwrap_or_default();
    let kind = match status {
        100..=199 => "Informational response",
        300..=399 => "Redirect response",
        400..=499 => "Client error",
        500..=599 => "Server error",
        _ => "Invalid status code",
    };
    let url = url.unwrap_or_default();
    format!(
        "{kind} '{status} {reason}' for url '{url}'\nFor more information check: https://developer.mozilla.org/en-US/docs/Web/HTTP/Status/{status}"
    )
}

// rmcp wraps a failed transport send in ServiceError::TransportSend, so the status error can sit under it
fn status_head<'source>(source: &'source (dyn Error + 'static)) -> Option<&'source ResponseHead> {
    let error = source.downcast_ref::<SdkError>()?;
    if let SdkError::Service(ServiceError::TransportSend(send_error)) = error
        && let Some(inner) = send_error.error.downcast_ref::<SdkError>()
    {
        return status_from_sdk(inner);
    }
    status_from_sdk(error)
}

fn status_from_sdk(error: &SdkError) -> Option<&ResponseHead> {
    if let SdkError::Http(http) = error
        && let McpHttpError::Status(head) = http.as_ref()
    {
        return Some(head.as_ref());
    }
    None
}

// A cleanup that ran past its bound surfaces as asyncio.CancelledError in LiteLLM, so it takes the cancellation path
pub(crate) const fn is_cancellation<Source>(error: &SessionError<Source>) -> bool
where
    Source: Error + Send + Sync + 'static,
{
    matches!(
        error,
        SessionError::Cancelled | SessionError::CleanupTimedOut
    )
}

// LiteLLM checks type(e).__name__ for "Broken"; the Rust analogue is the source type's name, not its message
pub(crate) fn names_broken_connection<Source>(error: &SessionError<Source>) -> bool
where
    Source: Error + Send + Sync + 'static,
{
    match error {
        SessionError::Runtime(_) | SessionError::Stream(_) => {
            short_type_name::<Source>().contains("Broken")
        }
        SessionError::Protocol(_)
        | SessionError::Cancelled
        | SessionError::CleanupTimedOut
        | SessionError::CleanupTask(_)
        | SessionError::ReadTimeout(_)
        | SessionError::Configuration(_) => false,
    }
}
