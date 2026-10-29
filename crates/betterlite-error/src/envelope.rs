//! The shared client-facing error envelope, extracted from `betterlite-server`'s `error_body.rs`.
//!
//! Also carries the provider response-header forwarding rules from
//! `litellm/litellm_core_utils/llm_response_utils/get_headers.py` and the
//! `UNSAFE_PROXY_RESPONSE_HEADERS` filter from `litellm/proxy/common_request_processing.py`.

use std::collections::BTreeMap;

use crate::error::ErrorMetadata;

mod tests;

/// The three JSON shapes a `LiteLLM` `getattr(e, field, "None")` lookup produces.
///
/// The attribute is missing (the literal string `"None"`), present and `None` (JSON
/// `null`), or present with a string value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WireField {
    Missing,
    Null,
    Text(String),
}

impl WireField {
    #[must_use]
    pub const fn text(value: String) -> Self {
        Self::Text(value)
    }
}

impl serde::Serialize for WireField {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            Self::Missing => serializer.serialize_str("None"),
            Self::Null => serializer.serialize_none(),
            Self::Text(text) => serializer.serialize_str(text),
        }
    }
}

/// The betterlite-owned, client-facing message for a classified error; never the
/// `Debug` dump of the underlying cause.
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

impl ErrorResponse {
    #[must_use]
    pub const fn new(error: ErrorObject) -> Self {
        Self { error }
    }
}

impl ErrorObject {
    #[must_use]
    pub const fn new(
        message: String,
        error_type: WireField,
        param: WireField,
        code: WireField,
    ) -> Self {
        Self {
            message,
            error_type,
            param,
            code,
        }
    }

    #[must_use]
    pub fn from_metadata(metadata: &ErrorMetadata, message: String) -> Self {
        Self::new(
            message,
            metadata.error_type.clone(),
            metadata.param.clone(),
            metadata.code.clone(),
        )
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

/// Narrower than [`HTTP_FRAMING_HEADERS`] / [`BROWSER_SECURITY_HEADERS`]: the subset that
/// also gets dropped in its `llm_provider-` prefixed form, per the B184 security deviation.
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

/// `get_response_headers`: forward the four bare `x-ratelimit` headers as-is, and
/// reprefix everything else `llm_provider-{name}` unless it already contains that marker.
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

fn is_unsafe_response_header(name_lower: &str) -> bool {
    HTTP_FRAMING_HEADERS.contains(&name_lower) || BROWSER_SECURITY_HEADERS.contains(&name_lower)
}

fn is_unsafe_prefixed_header(name_lower: &str) -> bool {
    name_lower
        .strip_prefix(LLM_PROVIDER_PREFIX)
        .is_some_and(|rest| COOKIE_AND_CORS_HEADERS.contains(&rest))
}

/// `UNSAFE_PROXY_RESPONSE_HEADERS` filtering, plus the raw-and-prefixed cookie/CORS
/// deviation: a malicious upstream cannot plant a `Set-Cookie` or CORS override under
/// either name.
#[must_use]
pub fn filter_unsafe_headers(headers: BTreeMap<String, String>) -> BTreeMap<String, String> {
    headers
        .into_iter()
        .filter(|(name, _)| {
            let lower = name.to_lowercase();
            !is_unsafe_response_header(&lower) && !is_unsafe_prefixed_header(&lower)
        })
        .collect()
}

/// The full header pipeline for one error.
///
/// `explicit_headers` (a `headers=` kwarg that bypasses projection, e.g. `RateLimitError`)
/// takes precedence over `response_headers` (projected through [`project_provider_headers`]
/// first), then both paths go through the same safety filter.
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
