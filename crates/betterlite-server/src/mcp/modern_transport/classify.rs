use axum::body::Body;
use axum::http::{HeaderMap, Response};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use rmcp::model::{
    ClientRequest, ErrorCode, GetMeta as _, JsonRpcRequest, ProtocolVersion, RequestId,
};

use super::{
    MCP_PROTOCOL_VERSION_HEADER, MODERN_PROTOCOL_VERSIONS, plain_error_response,
    unsupported_version_response,
};

const MCP_METHOD_HEADER: &str = "mcp-method";
const MCP_NAME_HEADER: &str = "mcp-name";
const ROUTING_HEADERS: [&str; 3] = [
    MCP_PROTOCOL_VERSION_HEADER,
    MCP_METHOD_HEADER,
    MCP_NAME_HEADER,
];
const VERSION_HEADER_MISMATCH: &str =
    "mcp-protocol-version header does not match the request envelope's protocol version";
const METHOD_HEADER_MISMATCH: &str = "mcp-method header does not match the request body's method";
const BASE64_PREFIX: &str = "=?base64?";
const BASE64_SUFFIX: &str = "?=";

pub(super) enum Rejection {
    InvalidParams(String),
    HeaderMismatch(String),
    UnsupportedVersion(String),
}

impl Rejection {
    pub(super) fn respond(self, id: Option<&RequestId>) -> Response<Body> {
        match self {
            Self::InvalidParams(message) => {
                plain_error_response(id, ErrorCode::INVALID_PARAMS, &message)
            }
            Self::HeaderMismatch(message) => {
                plain_error_response(id, ErrorCode::HEADER_MISMATCH, &message)
            }
            Self::UnsupportedVersion(requested) => unsupported_version_response(id, &requested),
        }
    }
}

pub(super) fn duplicated_routing_header(headers: &HeaderMap) -> Option<&'static str> {
    ROUTING_HEADERS
        .into_iter()
        .find(|name| headers.get_all(*name).iter().count() > 1)
}

pub(super) fn classify(
    request: &JsonRpcRequest<ClientRequest>,
    headers: &HeaderMap,
) -> Result<(), Rejection> {
    let meta = request.request.get_meta();
    let missing = meta.missing_required_keys(&ProtocolVersion::V_2026_07_28);
    let Some(version) = meta.protocol_version() else {
        return Err(Rejection::InvalidParams(missing_keys(&missing)));
    };
    if !missing.is_empty() {
        return Err(Rejection::InvalidParams(missing_keys(&missing)));
    }
    // the header rung runs before the version rung, so a client that disagrees with itself is told so
    let header_version = headers
        .get(MCP_PROTOCOL_VERSION_HEADER)
        .and_then(|value| value.to_str().ok());
    if header_version != Some(version.as_str()) {
        return Err(Rejection::HeaderMismatch(
            VERSION_HEADER_MISMATCH.to_owned(),
        ));
    }
    let method = request.request.method();
    let header_method = headers
        .get(MCP_METHOD_HEADER)
        .and_then(|value| value.to_str().ok());
    if header_method != Some(method) {
        return Err(Rejection::HeaderMismatch(METHOD_HEADER_MISMATCH.to_owned()));
    }
    if let Some((key, body_value)) = named_parameter(&request.request) {
        let header_name = headers
            .get(MCP_NAME_HEADER)
            .and_then(|value| value.to_str().ok());
        let decoded = decode_header_value(header_name);
        if decoded.as_deref() != Some(body_value) {
            return Err(Rejection::HeaderMismatch(format!(
                "mcp-name header does not match the request body's '{key}' parameter"
            )));
        }
    }
    if !MODERN_PROTOCOL_VERSIONS.contains(&version.as_str()) {
        return Err(Rejection::UnsupportedVersion(version.to_string()));
    }
    Ok(())
}

fn missing_keys(missing: &[&'static str]) -> String {
    format!(
        "params._meta is missing the required envelope key(s): {}",
        missing.join(", ")
    )
}

fn named_parameter(request: &ClientRequest) -> Option<(&'static str, &str)> {
    if let ClientRequest::CallToolRequest(call) = request {
        return Some(("name", call.params.name.as_ref()));
    }
    if let ClientRequest::GetPromptRequest(get) = request {
        return Some(("name", get.params.name.as_str()));
    }
    if let ClientRequest::ReadResourceRequest(read) = request {
        return Some(("uri", read.params.uri.as_str()));
    }
    None
}

fn decode_header_value(value: Option<&str>) -> Option<String> {
    let text = value?;
    let Some(payload) = text
        .strip_prefix(BASE64_PREFIX)
        .and_then(|rest| rest.strip_suffix(BASE64_SUFFIX))
    else {
        return Some(text.to_owned());
    };
    let decoded = STANDARD.decode(payload).ok()?;
    if STANDARD.encode(&decoded) != payload {
        return None;
    }
    String::from_utf8(decoded).ok()
}
