use betterlite_mcp::client::http_transport::error::McpHttpError;
use betterlite_mcp::client::sdk_transport::error::SdkError;
use betterlite_mcp::client::sessions::error::SessionError;
use betterlite_mcp::tools::pagination::ToolError;
use betterlite_prompts::arguments::json::{Json, Object};
use serde::Serialize;

// The Python class names LiteLLM's MCP failures carry; B8082 maps them back to its exception classes
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub(crate) enum McpErrorKind {
    McpError,
    TimeoutError,
    RuntimeError,
    ValueError,
    CancelledError,
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
    pub(crate) fn plain(kind: McpErrorKind, message: String) -> Self {
        Self {
            kind,
            message,
            code: None,
            data: None,
        }
    }

    fn mcp(code: i32, message: &str) -> Self {
        Self {
            kind: McpErrorKind::McpError,
            message: message.to_owned(),
            code: Some(code),
            data: None,
        }
    }
}

impl From<url::ParseError> for NativeMcpErrorPayload {
    fn from(error: url::ParseError) -> Self {
        Self::plain(
            McpErrorKind::ValueError,
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
            NativeMcpErrorPayload::plain(McpErrorKind::TimeoutError, timeout.message.clone())
        }
        SessionError::Runtime(source) => sdk_payload(source),
        SessionError::Stream(failure) => sdk_payload(failure.source_error()),
        SessionError::Cancelled | SessionError::CleanupTimedOut => {
            NativeMcpErrorPayload::plain(McpErrorKind::CancelledError, error.to_string())
        }
        SessionError::CleanupTask(cause) => {
            NativeMcpErrorPayload::plain(McpErrorKind::RuntimeError, cause.to_string())
        }
        SessionError::Configuration(config) => {
            NativeMcpErrorPayload::plain(McpErrorKind::ValueError, config.to_string())
        }
    }
}

pub(crate) fn tool_failure(error: &ToolError<SessionError<SdkError>>) -> NativeMcpErrorPayload {
    match error {
        ToolError::Session(inner) => session_payload(inner),
        ToolError::Timeout(_) => {
            NativeMcpErrorPayload::plain(McpErrorKind::TimeoutError, error.to_string())
        }
    }
}

fn sdk_payload(error: &SdkError) -> NativeMcpErrorPayload {
    match error {
        SdkError::Http(http) => http_payload(http),
        SdkError::JsonRpc(data) => NativeMcpErrorPayload::mcp(data.code.0, data.message.as_ref()),
        other => NativeMcpErrorPayload::plain(McpErrorKind::RuntimeError, other.to_string()),
    }
}

fn http_payload(error: &McpHttpError) -> NativeMcpErrorPayload {
    match error {
        McpHttpError::Status(head) => {
            let preview = head.capture.as_ref().map(|record| record.preview.as_str());
            status_payload(head.status, preview)
        }
        McpHttpError::Timeout(_) | McpHttpError::HeadTimeout(_) | McpHttpError::BodyTimeout(_) => {
            NativeMcpErrorPayload::plain(McpErrorKind::TimeoutError, error.to_string())
        }
        other => NativeMcpErrorPayload::plain(McpErrorKind::RuntimeError, other.to_string()),
    }
}

// The status and the masked body preview are all Python gets about an HTTP failure that carried no JSON-RPC error
pub(crate) fn status_payload(status: u16, preview: Option<&str>) -> NativeMcpErrorPayload {
    let number = Json::Number(serde_json::Number::from(status));
    let body = Json::Text(preview.unwrap_or("(no body captured)").to_owned());
    let mut data = Object::new();
    data.insert("status".to_owned(), number);
    data.insert("body".to_owned(), body);
    NativeMcpErrorPayload {
        kind: McpErrorKind::RuntimeError,
        message: format!("the MCP server answered HTTP {status}"),
        code: None,
        data: Some(Json::Object(data)),
    }
}
