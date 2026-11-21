use betterlite_anthropic::credentials::is_anthropic_oauth_key;
use indexmap::IndexMap;
use secrecy::SecretString;

use crate::api::request_headers::{ForwardedHeaders, HeaderAuth, HeaderValue};

pub const REDACTED_HEADER_VALUE: &str = "***REDACTED***";
pub const AUTHORIZATION: &str = "authorization";
const PROVIDER_KEY_HEADER: &str = "x-api-key";
const PROXY_KEY_HEADER: &str = "x-litellm-api-key";
// SpecialHeaders: every header the proxy reads a key or an MCP grant from
const SPECIAL_HEADERS: [&str; 9] = [
    AUTHORIZATION,
    "api-key",
    PROVIDER_KEY_HEADER,
    "x-goog-api-key",
    "ocp-apim-subscription-key",
    PROXY_KEY_HEADER,
    "x-mcp-auth",
    "x-mcp-servers",
    "x-mcp-access-groups",
];
// the MCP grants are request context, so only the key headers and the session ones count as credentials
const CREDENTIAL_HEADERS: [&str; 8] = [
    AUTHORIZATION,
    "api-key",
    PROVIDER_KEY_HEADER,
    "x-goog-api-key",
    "ocp-apim-subscription-key",
    PROXY_KEY_HEADER,
    "cookie",
    "proxy-authorization",
];

#[must_use]
pub fn clean_headers(headers: &IndexMap<String, String>, auth: &HeaderAuth) -> ForwardedHeaders {
    let mut cleaned = ForwardedHeaders::new();
    for (name, value) in headers {
        let lower = name.to_lowercase();
        if !keeps_header(&lower, value, auth) {
            continue;
        }
        let kept = if CREDENTIAL_HEADERS.contains(&lower.as_str()) {
            HeaderValue::Credential(SecretString::from(value.clone()))
        } else {
            HeaderValue::Plain(value.clone())
        };
        cleaned.insert(name.clone(), kept);
    }
    cleaned
}

#[must_use]
pub fn logged_headers(headers: &ForwardedHeaders) -> IndexMap<String, String> {
    headers
        .iter()
        .map(|(name, value)| {
            let logged = match value {
                HeaderValue::Plain(text) => text.clone(),
                HeaderValue::Credential(_) => REDACTED_HEADER_VALUE.to_owned(),
            };
            (name.clone(), logged)
        })
        .collect()
}

// an Anthropic OAuth token is the caller's own provider credential unless the proxy key came in the same header
fn keeps_header(lower: &str, value: &str, auth: &HeaderAuth) -> bool {
    let authenticated_with = |header: &str| {
        auth.authenticated_with_header
            .as_deref()
            .is_some_and(|used| used.to_lowercase() == header)
    };
    let is_custom_key = auth
        .litellm_key_header_name
        .as_deref()
        .is_some_and(|custom| custom.to_lowercase() == lower);
    if lower == AUTHORIZATION && is_anthropic_oauth_key(Some(value)) {
        return !authenticated_with(AUTHORIZATION);
    }
    if lower == PROVIDER_KEY_HEADER {
        return auth.forward_llm_provider_auth_headers && !authenticated_with(PROVIDER_KEY_HEADER);
    }
    let special = SPECIAL_HEADERS.contains(&lower);
    if auth.forward_llm_provider_auth_headers && special {
        return !is_custom_key && lower != AUTHORIZATION && lower != PROXY_KEY_HEADER;
    }
    !special && !is_custom_key
}
