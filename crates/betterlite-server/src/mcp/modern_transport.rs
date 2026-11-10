use axum::body::{Body, to_bytes};
use axum::http::{HeaderMap, HeaderValue, Method, Request, Response, StatusCode, header};
use rmcp::ServerHandler;
use rmcp::model::{ClientJsonRpcMessage, ClientRequest, ErrorCode, JsonRpcRequest, RequestId};
use serde::Serialize;

use crate::mcp::stream_transport::wire;

mod classify;
mod exchange;
mod tests;

const HANDSHAKE_PROTOCOL_VERSIONS: [&str; 4] =
    ["2024-11-05", "2025-03-26", "2025-06-18", "2025-11-25"];
pub(super) const MODERN_PROTOCOL_VERSIONS: [&str; 1] = ["2026-07-28"];
pub(super) const MCP_PROTOCOL_VERSION_HEADER: &str = "mcp-protocol-version";
const CONTENT_TYPE_JSON: &str = "application/json";
pub(super) const INVALID_BODY_MESSAGE: &str =
    "Body must be a single JSON-RPC request or notification object";
// MCP's DEFAULT_MAX_REQUEST_BODY_SIZE, the body cap the SDK's middleware applies
const MAX_REQUEST_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Default)]
pub struct ModernTransport;

impl ModernTransport {
    pub async fn handle<H>(&self, request: Request<Body>, handler: H) -> Response<Body>
    where
        H: ServerHandler,
    {
        if *request.method() != Method::POST {
            return method_not_allowed();
        }
        let (parts, body) = request.into_parts();
        // the SDK's security middleware always checks a POST's Content-Type, whatever its other settings
        if !has_json_content_type(&parts.headers) {
            let mut response = Response::new(Body::from("Invalid Content-Type header"));
            *response.status_mut() = StatusCode::BAD_REQUEST;
            return response;
        }
        if !accepts_both_media_types(&parts.headers) {
            return empty_response(StatusCode::NOT_ACCEPTABLE);
        }
        let Ok(bytes) = to_bytes(body, MAX_REQUEST_BYTES).await else {
            return body_too_large();
        };
        let message = match wire::decode_message(&bytes) {
            Ok(message) => message,
            Err(wire::DecodeError::Parse { .. }) => {
                return error_response(None, ErrorCode::PARSE_ERROR, "Parse error", None);
            }
            Err(wire::DecodeError::Validation { .. }) => {
                return classify::decode_refusal(&bytes);
            }
        };
        match message {
            ClientJsonRpcMessage::Request(call) => {
                request_answer(call, &parts.headers, handler).await
            }
            ClientJsonRpcMessage::Notification(_) => acknowledge_notification(&parts.headers),
            ClientJsonRpcMessage::Response(_) | ClientJsonRpcMessage::Error(_) => {
                error_response(None, ErrorCode::INVALID_REQUEST, INVALID_BODY_MESSAGE, None)
            }
        }
    }
}

async fn request_answer<H>(
    call: JsonRpcRequest<ClientRequest>,
    headers: &HeaderMap,
    handler: H,
) -> Response<Body>
where
    H: ServerHandler,
{
    // LiteLLM's entry refuses every version outside its configured list before this path runs
    if let Some(name) = classify::duplicated_routing_header(headers) {
        return classify::Rejection::HeaderMismatch(format!(
            "{name} header appears more than once"
        ))
        .respond(Some(&call.id));
    }
    if let Err(rejection) = classify::check_envelope_and_headers(&call, headers) {
        return rejection.respond(Some(&call.id));
    }
    if let Err(rejection) = classify::check_method_surface(&call) {
        return rejection.respond(Some(&call.id));
    }
    exchange::serve_request(call, handler).await
}

#[must_use]
pub fn takes_modern_path(headers: &HeaderMap) -> bool {
    headers
        .get(MCP_PROTOCOL_VERSION_HEADER)
        .is_some_and(|value| {
            // ASGI reads the header as latin-1, so a value that is not UTF-8 is outside the handshake set too
            value.to_str().map_or(true, |version| {
                !HANDSHAKE_PROTOCOL_VERSIONS.contains(&version)
            })
        })
}

fn acknowledge_notification(headers: &HeaderMap) -> Response<Body> {
    let requested = headers
        .get(MCP_PROTOCOL_VERSION_HEADER)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    if !MODERN_PROTOCOL_VERSIONS.contains(&requested) {
        return classify::Rejection::UnsupportedVersion(requested.to_owned()).respond(None);
    }
    empty_response(StatusCode::ACCEPTED)
}

fn has_json_content_type(headers: &HeaderMap) -> bool {
    headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.to_ascii_lowercase().starts_with(CONTENT_TYPE_JSON))
}

fn accepts_both_media_types(headers: &HeaderMap) -> bool {
    headers
        .get(header::ACCEPT)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|accept| {
            let mut json = false;
            let mut event_stream = false;
            for entry in accept.split(',') {
                let media = entry
                    .split(';')
                    .next()
                    .unwrap_or_default()
                    .trim()
                    .to_ascii_lowercase();
                match media.as_str() {
                    "*/*" => return true,
                    "application/json" | "application/*" => json = true,
                    "text/event-stream" | "text/*" => event_stream = true,
                    _ => {}
                }
            }
            json && event_stream
        })
}

fn method_not_allowed() -> Response<Body> {
    let mut response = empty_response(StatusCode::METHOD_NOT_ALLOWED);
    response
        .headers_mut()
        .insert(header::ALLOW, HeaderValue::from_static("POST"));
    response
}

fn body_too_large() -> Response<Body> {
    let mut response = Response::new(Body::from("Request body too large"));
    *response.status_mut() = StatusCode::PAYLOAD_TOO_LARGE;
    response
}

pub(super) fn error_response(
    id: Option<&RequestId>,
    code: ErrorCode,
    message: &str,
    data: Option<RejectionData<'_>>,
) -> Response<Body> {
    let body = ErrorBody {
        jsonrpc: "2.0",
        id,
        error: ErrorDetail {
            code,
            message,
            data,
        },
    };
    let payload = serde_json::to_string(&body).unwrap_or_default();
    json_response(error_status(code), payload)
}

pub(super) fn json_response(status: StatusCode, payload: String) -> Response<Body> {
    let mut response = Response::new(Body::from(payload));
    *response.status_mut() = status;
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(CONTENT_TYPE_JSON),
    );
    response
}

pub(super) fn empty_response(status: StatusCode) -> Response<Body> {
    let mut response = Response::new(Body::empty());
    *response.status_mut() = status;
    response
}

pub(super) const fn error_status(code: ErrorCode) -> StatusCode {
    match code {
        // the missing-client-capability code (-32021) is the sixth 400 in the SDK table
        ErrorCode::PARSE_ERROR
        | ErrorCode::INVALID_REQUEST
        | ErrorCode::INVALID_PARAMS
        | ErrorCode::HEADER_MISMATCH
        | ErrorCode::UNSUPPORTED_PROTOCOL_VERSION
        | ErrorCode(-32021) => StatusCode::BAD_REQUEST,
        ErrorCode::METHOD_NOT_FOUND => StatusCode::NOT_FOUND,
        // the SDK leaves INTERNAL_ERROR and every unmapped code at 200
        _ => StatusCode::OK,
    }
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
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<RejectionData<'text>>,
}

#[derive(Serialize)]
#[serde(untagged)]
pub(super) enum RejectionData<'text> {
    UnsupportedVersion(UnsupportedVersionData<'text>),
    Text(&'text str),
}

#[derive(Serialize)]
pub(super) struct UnsupportedVersionData<'text> {
    supported: &'text [&'text str],
    requested: &'text str,
}
