use std::time::Duration;

use axum::body::{Body, Bytes, to_bytes};
use axum::http::{HeaderMap, HeaderValue, Request, Response, StatusCode, header};
use rmcp::model::{ClientJsonRpcMessage, ErrorCode, RequestId, ServerJsonRpcMessage};
use serde::Serialize;
use serde_json::error::Category;

pub const MCP_SESSION_ID_HEADER: &str = "mcp-session-id";

const CONTENT_TYPE_JSON: &str = "application/json";
const CONTENT_TYPE_SSE: &str = "text/event-stream";
// MCP's DEFAULT_MAX_REQUEST_BODY_SIZE, enforced by the SDK's request body middleware
const MAX_REQUEST_BYTES: usize = 4 * 1024 * 1024;
// sse-starlette's DEFAULT_PING_INTERVAL
pub(super) const PING_INTERVAL: Duration = Duration::from_secs(15);

#[derive(Debug, thiserror::Error)]
pub enum DecodeError {
    #[error("Parse error: {source}")]
    Parse {
        #[source]
        source: serde_json::Error,
    },
    #[error("Validation error: {source}")]
    Validation {
        #[source]
        source: serde_json::Error,
    },
}

#[derive(Serialize)]
struct ErrorBody<'text> {
    jsonrpc: &'static str,
    id: Option<&'text RequestId>,
    error: ErrorDetail<'text>,
}

#[derive(Serialize)]
struct ErrorDetail<'text> {
    code: ErrorCode,
    message: &'text str,
}

pub fn decode_message(bytes: &[u8]) -> Result<ClientJsonRpcMessage, DecodeError> {
    serde_json::from_slice(bytes).map_err(|source| match source.classify() {
        Category::Data => DecodeError::Validation { source },
        Category::Io | Category::Syntax | Category::Eof => DecodeError::Parse { source },
    })
}

pub(super) async fn decode_post(
    request: Request<Body>,
    session_id: Option<&str>,
) -> Result<ClientJsonRpcMessage, Box<Response<Body>>> {
    let (has_json, has_sse) = accepted_media_types(request.headers());
    if !has_json || !has_sse {
        return Err(Box::new(error_response(
            StatusCode::NOT_ACCEPTABLE,
            ErrorCode::INVALID_REQUEST,
            "Not Acceptable: Client must accept both application/json and text/event-stream",
            session_id,
        )));
    }
    if !is_json_content_type(request.headers()) {
        return Err(Box::new(error_response(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            ErrorCode::INVALID_REQUEST,
            "Unsupported Media Type: Content-Type must be application/json",
            session_id,
        )));
    }
    let Ok(bytes) = to_bytes(request.into_body(), MAX_REQUEST_BYTES).await else {
        return Err(Box::new(body_too_large()));
    };
    decode_message(&bytes).map_err(|error| Box::new(decode_error_response(&error, session_id)))
}

pub(super) fn accepted_media_types(headers: &HeaderMap) -> (bool, bool) {
    let Some(accept) = headers
        .get(header::ACCEPT)
        .and_then(|value| value.to_str().ok())
    else {
        return (false, false);
    };
    let mut has_json = false;
    let mut has_sse = false;
    for entry in accept.split(',') {
        let media_type = entry
            .split(';')
            .next()
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase();
        match media_type.as_str() {
            "*/*" => {
                has_json = true;
                has_sse = true;
            }
            "application/json" | "application/*" => has_json = true,
            "text/event-stream" | "text/*" => has_sse = true,
            _ => {}
        }
    }
    (has_json, has_sse)
}

fn is_json_content_type(headers: &HeaderMap) -> bool {
    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    content_type
        .split(';')
        .next()
        .unwrap_or_default()
        .split(',')
        .any(|part| part.trim() == CONTENT_TYPE_JSON)
}

fn decode_error_response(error: &DecodeError, session_id: Option<&str>) -> Response<Body> {
    let (status, code) = match error {
        DecodeError::Parse { .. } => (StatusCode::BAD_REQUEST, ErrorCode::PARSE_ERROR),
        DecodeError::Validation { .. } => (StatusCode::BAD_REQUEST, ErrorCode::INVALID_PARAMS),
    };
    error_response(status, code, &error.to_string(), session_id)
}

pub(super) fn error_response(
    status: StatusCode,
    code: ErrorCode,
    message: &str,
    session_id: Option<&str>,
) -> Response<Body> {
    let body = ErrorBody {
        jsonrpc: "2.0",
        id: None,
        error: ErrorDetail { code, message },
    };
    let payload = serde_json::to_string(&body).unwrap_or_default();
    json_body(status, Body::from(payload), session_id)
}

pub(super) fn json_body(
    status: StatusCode,
    payload: Body,
    session_id: Option<&str>,
) -> Response<Body> {
    let mut response = Response::new(payload);
    *response.status_mut() = status;
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(CONTENT_TYPE_JSON),
    );
    insert_session_id(&mut response, session_id);
    response
}

pub(super) fn method_not_allowed(session_id: Option<&str>) -> Response<Body> {
    let mut response = error_response(
        StatusCode::METHOD_NOT_ALLOWED,
        ErrorCode::INVALID_REQUEST,
        "Method Not Allowed",
        session_id,
    );
    response
        .headers_mut()
        .insert(header::ALLOW, HeaderValue::from_static("GET, POST, DELETE"));
    response
}

fn body_too_large() -> Response<Body> {
    let mut response = Response::new(Body::from("Request body too large"));
    *response.status_mut() = StatusCode::PAYLOAD_TOO_LARGE;
    response
}

pub(super) fn sse_response(body: Body, session_id: Option<&str>) -> Response<Body> {
    let mut response = Response::new(body);
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(CONTENT_TYPE_SSE),
    );
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("no-cache, no-transform"),
    );
    headers.insert(header::CONNECTION, HeaderValue::from_static("keep-alive"));
    headers.insert(
        header::HeaderName::from_static("x-accel-buffering"),
        HeaderValue::from_static("no"),
    );
    insert_session_id(&mut response, session_id);
    response
}

fn insert_session_id(response: &mut Response<Body>, session_id: Option<&str>) {
    let Some(id) = session_id else {
        return;
    };
    let Ok(header_value) = HeaderValue::from_str(id) else {
        return;
    };
    response.headers_mut().insert(
        header::HeaderName::from_static(MCP_SESSION_ID_HEADER),
        header_value,
    );
}

#[must_use]
pub fn sse_event(message: &ServerJsonRpcMessage) -> Bytes {
    let json = serde_json::to_string(message).unwrap_or_else(|error| {
        tracing::error!(%error, "an MCP server message could not be serialized for SSE");
        String::new()
    });
    Bytes::from(format!("event: message\r\ndata: {json}\r\n\r\n"))
}

pub(super) fn ping() -> Bytes {
    let now = chrono::Utc::now();
    Bytes::from(format!(
        ": ping - {}\r\n\r\n",
        now.format("%Y-%m-%d %H:%M:%S%.6f+00:00")
    ))
}

mod tests;
