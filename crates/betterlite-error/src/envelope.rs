use std::collections::BTreeMap;

use crate::error::ErrorMetadata;

mod tests;

// the externally tagged default: a fieldless variant serializes to its own name as a bare string
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum NoneMarker {
    #[serde(rename = "None")]
    None,
}

// mirrors getattr(e, field, "None"): attribute missing, present and null, or present with text
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(untagged)]
pub enum WireField {
    Missing(NoneMarker),
    Null,
    Text(String),
}

impl WireField {
    pub const MISSING: Self = Self::Missing(NoneMarker::None);
}

// never the Debug dump of the underlying cause
#[derive(Debug, Clone, serde::Serialize)]
pub struct ErrorObject {
    pub message: String,
    #[serde(rename = "type")]
    pub error_type: WireField,
    pub param: WireField,
    pub code: WireField,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ErrorResponse {
    pub error: ErrorObject,
}

impl ErrorObject {
    #[must_use]
    pub fn from_metadata(metadata: &ErrorMetadata, message: String) -> Self {
        let code = metadata.code.clone();
        Self {
            message,
            error_type: metadata.error_type.clone(),
            param: metadata.param.clone(),
            code,
        }
    }
}

const BARE_RATE_LIMIT_HEADERS: [&str; 4] = [
    "x-ratelimit-limit-requests",
    "x-ratelimit-remaining-requests",
    "x-ratelimit-limit-tokens",
    "x-ratelimit-remaining-tokens",
];

const HTTP_FRAMING_HEADERS: [&str; 8] = [
    "content-length",
    "transfer-encoding",
    "content-encoding",
    "content-type",
    "set-cookie",
    "cookie",
    "proxy-authenticate",
    "proxy-authorization",
];

const BROWSER_SECURITY_HEADERS: [&str; 13] = [
    "access-control-allow-origin",
    "access-control-allow-credentials",
    "access-control-allow-methods",
    "access-control-allow-headers",
    "access-control-expose-headers",
    "content-security-policy",
    "content-security-policy-report-only",
    "clear-site-data",
    "strict-transport-security",
    "x-frame-options",
    "cross-origin-opener-policy",
    "cross-origin-embedder-policy",
    "cross-origin-resource-policy",
];

// the B184 deviation: also dropped once llm_provider- prefixed, unlike the rest of HTTP_FRAMING_HEADERS/BROWSER_SECURITY_HEADERS
const COOKIE_AND_CORS_HEADERS: [&str; 7] = [
    "set-cookie",
    "cookie",
    "access-control-allow-origin",
    "access-control-allow-credentials",
    "access-control-allow-methods",
    "access-control-allow-headers",
    "access-control-expose-headers",
];

const LLM_PROVIDER_MARKER: &str = "llm_provider";
const LLM_PROVIDER_PREFIX: &str = "llm_provider-";

// get_response_headers: the four bare x-ratelimit headers forward as-is, everything else gets the llm_provider- prefix
#[must_use]
pub fn project_provider_headers(
    response_headers: &BTreeMap<String, String>,
) -> BTreeMap<String, String> {
    let mut projected = BTreeMap::new();
    for (name, value) in response_headers {
        if name.contains(LLM_PROVIDER_MARKER) {
            projected.insert(name.clone(), value.clone());
        } else {
            projected.insert(format!("{LLM_PROVIDER_PREFIX}{name}"), value.clone());
        }
    }
    for name in BARE_RATE_LIMIT_HEADERS {
        if let Some(value) = response_headers.get(name) {
            projected.insert(name.to_owned(), value.clone());
        }
    }
    projected
}

// UNSAFE_PROXY_RESPONSE_HEADERS, plus the raw-and-prefixed cookie/CORS deviation
#[must_use]
pub fn filter_unsafe_headers(headers: BTreeMap<String, String>) -> BTreeMap<String, String> {
    let mut kept = BTreeMap::new();
    for (name, value) in headers {
        let lower = name.to_lowercase();
        let unsafe_raw = HTTP_FRAMING_HEADERS.contains(&lower.as_str())
            || BROWSER_SECURITY_HEADERS.contains(&lower.as_str());
        let unsafe_prefixed = lower
            .strip_prefix(LLM_PROVIDER_PREFIX)
            .is_some_and(|rest| COOKIE_AND_CORS_HEADERS.contains(&rest));
        if !unsafe_raw && !unsafe_prefixed {
            kept.insert(name, value);
        }
    }
    kept
}

// explicit_headers (a headers= kwarg, e.g. RateLimitError) bypasses projection; response_headers goes through project_provider_headers first
#[must_use]
pub fn allowed_response_headers(
    explicit_headers: Option<&BTreeMap<String, String>>,
    response_headers: &BTreeMap<String, String>,
) -> BTreeMap<String, String> {
    let raw = explicit_headers
        .cloned()
        .unwrap_or_else(|| project_provider_headers(response_headers));
    filter_unsafe_headers(raw)
}
