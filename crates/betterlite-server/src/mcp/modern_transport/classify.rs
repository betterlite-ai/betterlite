use axum::body::Body;
use axum::http::{HeaderMap, Response};
use base64::Engine as _;
use base64::alphabet;
use base64::engine::general_purpose::{GeneralPurpose, GeneralPurposeConfig};
use rmcp::model::{
    ClientRequest, CustomRequest, ErrorCode, GetMeta as _, JsonRpcRequest, RequestId,
    RequestMetaObject,
};
use serde::Deserialize;
use serde::de::IgnoredAny;

use super::{
    INVALID_BODY_MESSAGE, MCP_PROTOCOL_VERSION_HEADER, MODERN_PROTOCOL_VERSIONS, RejectionData,
    UnsupportedVersionData, error_response,
};

const MCP_METHOD_HEADER: &str = "mcp-method";
const MCP_NAME_HEADER: &str = "mcp-name";
const PROTOCOL_VERSION_KEY: &str = "io.modelcontextprotocol/protocolVersion";
const CLIENT_CAPABILITIES_KEY: &str = "io.modelcontextprotocol/clientCapabilities";
const ROUTING_HEADERS: [&str; 3] = [
    MCP_PROTOCOL_VERSION_HEADER,
    MCP_METHOD_HEADER,
    MCP_NAME_HEADER,
];
const VERSION_HEADER_MISMATCH: &str =
    "mcp-protocol-version header does not match the request envelope's protocol version";
const METHOD_HEADER_MISMATCH: &str = "mcp-method header does not match the request body's method";
const META_OBJECT_MESSAGE: &str = "params._meta must be an object carrying the required 'io.modelcontextprotocol/protocolVersion' and 'io.modelcontextprotocol/clientCapabilities' envelope keys";
const INVALID_PARAMETERS_MESSAGE: &str = "Invalid request parameters";
const BASE64_PREFIX: &str = "=?base64?";
const BASE64_SUFFIX: &str = "?=";
// Python's b64decode(validate=True) tolerates non-canonical trailing bits, so the re-encode check rejects them
const HEADER_DECODER: GeneralPurpose = GeneralPurpose::new(
    &alphabet::STANDARD,
    GeneralPurposeConfig::new().with_decode_allow_trailing_bits(true),
);
// the mcp_types.methods CLIENT_REQUESTS rows for 2026-07-28
const MODERN_CLIENT_METHODS: [&str; 10] = [
    "completion/complete",
    "prompts/get",
    "prompts/list",
    "resources/list",
    "resources/read",
    "resources/templates/list",
    "server/discover",
    "subscriptions/listen",
    "tools/call",
    "tools/list",
];

pub(super) enum Rejection {
    InvalidParams(String),
    InvalidRequestParameters,
    MethodNotFound(String),
    HeaderMismatch(String),
    UnsupportedVersion(String),
}

impl Rejection {
    pub(super) fn respond(self, id: Option<&RequestId>) -> Response<Body> {
        match self {
            Self::InvalidParams(message) => {
                error_response(id, ErrorCode::INVALID_PARAMS, &message, None)
            }
            Self::InvalidRequestParameters => error_response(
                id,
                ErrorCode::INVALID_PARAMS,
                INVALID_PARAMETERS_MESSAGE,
                Some(RejectionData::Text("")),
            ),
            Self::MethodNotFound(method) => error_response(
                id,
                ErrorCode::METHOD_NOT_FOUND,
                "Method not found",
                Some(RejectionData::Text(&method)),
            ),
            Self::HeaderMismatch(message) => {
                error_response(id, ErrorCode::HEADER_MISMATCH, &message, None)
            }
            Self::UnsupportedVersion(requested) => error_response(
                id,
                ErrorCode::UNSUPPORTED_PROTOCOL_VERSION,
                "Unsupported protocol version",
                Some(RejectionData::UnsupportedVersion(UnsupportedVersionData {
                    supported: &MODERN_PROTOCOL_VERSIONS,
                    requested: &requested,
                })),
            ),
        }
    }
}

// a request rmcp could not decode at all, which the SDK's classifier still refuses by rung
pub(super) fn decode_refusal(body: &[u8]) -> Response<Body> {
    let envelope = match serde_json::from_slice::<Envelope>(body) {
        Ok(envelope) if envelope.jsonrpc == "2.0" => envelope,
        _ => return error_response(None, ErrorCode::INVALID_REQUEST, INVALID_BODY_MESSAGE, None),
    };
    error_response(
        Some(&envelope.id),
        ErrorCode::INVALID_PARAMS,
        META_OBJECT_MESSAGE,
        None,
    )
}

pub(super) fn duplicated_routing_header(headers: &HeaderMap) -> Option<&'static str> {
    ROUTING_HEADERS
        .into_iter()
        .find(|name| headers.get_all(*name).iter().count() > 1)
}

pub(super) fn check_envelope_and_headers(
    request: &JsonRpcRequest<ClientRequest>,
    headers: &HeaderMap,
) -> Result<(), Rejection> {
    let meta = request.request.get_meta();
    let missing: Vec<&'static str> = [PROTOCOL_VERSION_KEY, CLIENT_CAPABILITIES_KEY]
        .into_iter()
        .filter(|key| !meta.contains_key(*key))
        .collect();
    if !missing.is_empty() {
        return Err(Rejection::InvalidParams(missing_keys(&missing)));
    }
    let body_version = check_headers(&request.request, meta, headers)?;
    if !MODERN_PROTOCOL_VERSIONS.contains(&body_version) {
        return Err(Rejection::UnsupportedVersion(body_version.to_owned()));
    }
    Ok(())
}

fn check_headers<'meta>(
    request: &ClientRequest,
    meta: &'meta RequestMetaObject,
    headers: &HeaderMap,
) -> Result<&'meta str, Rejection> {
    // a header string can never equal a non-string body value, so that case is the header rung too
    let Some(body_version) = meta
        .get(PROTOCOL_VERSION_KEY)
        .and_then(|value| value.as_str())
    else {
        return Err(Rejection::HeaderMismatch(
            VERSION_HEADER_MISMATCH.to_owned(),
        ));
    };
    let header_version = headers
        .get(MCP_PROTOCOL_VERSION_HEADER)
        .and_then(|value| value.to_str().ok());
    if header_version != Some(body_version) {
        return Err(Rejection::HeaderMismatch(
            VERSION_HEADER_MISMATCH.to_owned(),
        ));
    }
    let header_method = headers
        .get(MCP_METHOD_HEADER)
        .and_then(|value| value.to_str().ok());
    if header_method != Some(request.method()) {
        return Err(Rejection::HeaderMismatch(METHOD_HEADER_MISMATCH.to_owned()));
    }
    check_name_header(request, headers)?;
    Ok(body_version)
}

fn check_name_header(request: &ClientRequest, headers: &HeaderMap) -> Result<(), Rejection> {
    if let Some((key, body_value)) = named_parameter(request) {
        let header_name = headers
            .get(MCP_NAME_HEADER)
            .and_then(|value| value.to_str().ok());
        if decode_header_value(header_name).as_deref() != Some(body_value) {
            return Err(name_mismatch(key));
        }
    }
    // a surface method whose params did not decode as their typed request still runs the name rung
    if let ClientRequest::CustomRequest(custom) = request
        && let Some(key) = name_key(request.method())
        && custom_name_mismatch(key, custom, headers)
    {
        return Err(name_mismatch(key));
    }
    Ok(())
}

fn name_mismatch(key: &str) -> Rejection {
    Rejection::HeaderMismatch(format!(
        "mcp-name header does not match the request body's '{key}' parameter"
    ))
}

// the SDK runs validate_client_request before handler lookup and answers -32601 for a method outside the surface
pub(super) fn check_method_surface(
    request: &JsonRpcRequest<ClientRequest>,
) -> Result<(), Rejection> {
    let method = request.request.method();
    if !MODERN_CLIENT_METHODS.contains(&method) {
        return Err(Rejection::MethodNotFound(method.to_owned()));
    }
    if matches!(&request.request, ClientRequest::CustomRequest(_)) {
        return Err(Rejection::InvalidRequestParameters);
    }
    Ok(())
}

fn missing_keys(missing: &[&'static str]) -> String {
    format!(
        "params._meta is missing the required envelope key(s): {}",
        missing.join(", ")
    )
}

fn name_key(method: &str) -> Option<&'static str> {
    match method {
        "tools/call" | "prompts/get" => Some("name"),
        "resources/read" => Some("uri"),
        _ => None,
    }
}

fn named_parameter(request: &ClientRequest) -> Option<(&'static str, &str)> {
    let key = name_key(request.method())?;
    if let ClientRequest::CallToolRequest(call) = request {
        return Some((key, call.params.name.as_ref()));
    }
    if let ClientRequest::GetPromptRequest(get) = request {
        return Some((key, get.params.name.as_str()));
    }
    if let ClientRequest::ReadResourceRequest(read) = request {
        return Some((key, read.params.uri.as_str()));
    }
    None
}

fn custom_name_mismatch(key: &str, custom: &CustomRequest, headers: &HeaderMap) -> bool {
    let Some(value) = custom.params.as_ref().and_then(|params| params.get(key)) else {
        return false;
    };
    if value.is_null() {
        return false;
    }
    let header_name = headers
        .get(MCP_NAME_HEADER)
        .and_then(|header| header.to_str().ok());
    let decoded = decode_header_value(header_name);
    // only a string equal to the decoded header matches; any other value, and an absent header, disagrees
    value
        .as_str()
        .is_none_or(|text| decoded.as_deref() != Some(text))
}

fn decode_header_value(value: Option<&str>) -> Option<String> {
    let text = value?;
    let Some(payload) = text
        .strip_prefix(BASE64_PREFIX)
        .and_then(|rest| rest.strip_suffix(BASE64_SUFFIX))
    else {
        return Some(text.to_owned());
    };
    let decoded = HEADER_DECODER.decode(payload).ok()?;
    if HEADER_DECODER.encode(&decoded) != payload {
        return None;
    }
    String::from_utf8(decoded).ok()
}

#[derive(Deserialize)]
struct Envelope {
    jsonrpc: String,
    id: RequestId,
    #[serde(rename = "method")]
    _method: String,
    #[serde(default)]
    _params: Option<EnvelopeParams>,
}

#[derive(Deserialize)]
struct EnvelopeParams {
    #[serde(rename = "_meta", default)]
    _meta: Option<IgnoredAny>,
}
