use betterlite_mcp::client::http_transport::error::McpHttpError;
use betterlite_mcp::client::sdk_transport::error::SdkError;
use betterlite_mcp::client::sessions::error::SessionError;
use betterlite_mcp::tools::pagination::ToolError;
use betterlite_prompts::arguments::json::{Json, Object};
use rmcp::service::ServiceError;
use serde::Serialize;
// The Python class names LiteLLM's MCP failures carry; B8082 maps them back to its exception classes
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub(crate) enum McpErrorKind {
    #[serde(rename = "McpError")]
    Mcp,
    #[serde(rename = "TimeoutError")]
    Timeout,
    #[serde(rename = "RuntimeError")]
    Runtime,
    #[serde(rename = "ValueError")]
    Value,
    #[serde(rename = "CancelledError")]
    Cancelled,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct NativeMcpErrorPayload {
    #[serde(rename = "type")]
    pub(crate) kind: McpErrorKind,
    pub(crate) message: String,
    pub(crate) code: Option<i32>,
    pub(crate) data: Option<Json>,
}
impl NativeMcpErrorPayload {
    pub(crate) const fn plain(kind: McpErrorKind, message: String) -> Self {
        Self {
            kind,
            message,
            code: None,
            data: None,
        }
    }

    fn mcp(code: i32, message: &str) -> Self {
        Self {
            kind: McpErrorKind::Mcp,
            message: message.to_owned(),
            code: Some(code),
            data: None,
        }
    }
}
impl From<url::ParseError> for NativeMcpErrorPayload {
    fn from(error: url::ParseError) -> Self {
        Self::plain(
            McpErrorKind::Value,
            format!("url is not a valid URL: {error}"),
        )
    }
}
// The JSON-RPC code stays as the server sent it, including -32022 for a rejected protocol version
pub(crate) fn session_payload(error: &SessionError<SdkError>) -> NativeMcpErrorPayload {
    match error {
        SessionError::Protocol(failure) => {
            NativeMcpErrorPayload::mcp(failure.code, failure.message.as_ref())
        }
        SessionError::ReadTimeout(timeout) => {
            NativeMcpErrorPayload::plain(McpErrorKind::Timeout, timeout.message.clone())
        }
        SessionError::Runtime(source) => sdk_payload(source),
        SessionError::Stream(failure) => sdk_payload(failure.source_error()),
        SessionError::Cancelled | SessionError::CleanupTimedOut => {
            NativeMcpErrorPayload::plain(McpErrorKind::Cancelled, error.to_string())
        }
        SessionError::CleanupTask(cause) => {
            NativeMcpErrorPayload::plain(McpErrorKind::Runtime, cause.to_string())
        }
        SessionError::Configuration(config) => {
            NativeMcpErrorPayload::plain(McpErrorKind::Value, config.to_string())
        }
    }
}
pub(crate) fn tool_failure(error: &ToolError<SessionError<SdkError>>) -> NativeMcpErrorPayload {
    match error {
        ToolError::Session(inner) => session_payload(inner),
        ToolError::Timeout(_) => {
            NativeMcpErrorPayload::plain(McpErrorKind::Timeout, error.to_string())
        }
    }
}
fn sdk_payload(error: &SdkError) -> NativeMcpErrorPayload {
    if let SdkError::Http(http) = error {
        return http_payload(http);
    }
    if let SdkError::JsonRpc(data) = error {
        return NativeMcpErrorPayload::mcp(data.code.0, data.message.as_ref());
    }
    if let SdkError::Service(service) = error {
        return service_payload(service);
    }
    NativeMcpErrorPayload::plain(McpErrorKind::Runtime, error.to_string())
}
// rmcp wraps a failed transport send in its ServiceError, so the timeout under it stays a TimeoutError
fn service_payload(error: &ServiceError) -> NativeMcpErrorPayload {
    if let ServiceError::Timeout { .. } = error {
        return NativeMcpErrorPayload::plain(McpErrorKind::Timeout, error.to_string());
    }
    if let ServiceError::TransportSend(dynamic) = error
        && let Some(source) = dynamic.error.downcast_ref::<SdkError>()
    {
        return sdk_payload(source);
    }
    NativeMcpErrorPayload::plain(McpErrorKind::Runtime, error.to_string())
}
fn http_payload(error: &McpHttpError) -> NativeMcpErrorPayload {
    if let McpHttpError::Status(head) = error {
        let preview = head.capture.as_ref().map(|record| record.preview.as_str());
        return status_payload(head.status, preview);
    }
    let timed_out = matches!(
        error,
        McpHttpError::Timeout(_) | McpHttpError::HeadTimeout(_) | McpHttpError::BodyTimeout(_)
    );
    let kind = if timed_out {
        McpErrorKind::Timeout
    } else {
        McpErrorKind::Runtime
    };
    NativeMcpErrorPayload::plain(kind, error.to_string())
}
// The status and the masked body preview are all Python gets about an HTTP failure that carried no JSON-RPC error
pub(crate) fn status_payload(status: u16, preview: Option<&str>) -> NativeMcpErrorPayload {
    let number = Json::Number(serde_json::Number::from(status));
    let body = Json::Text(preview.unwrap_or("(no body captured)").to_owned());
    let mut data = Object::new();
    data.insert("status".to_owned(), number);
    data.insert("body".to_owned(), body);
    NativeMcpErrorPayload {
        kind: McpErrorKind::Runtime,
        message: format!("the MCP server answered HTTP {status}"),
        code: None,
        data: Some(Json::Object(data)),
    }
}
