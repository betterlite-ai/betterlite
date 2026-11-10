use std::collections::BTreeMap;

use betterlite_mcp::credential_contracts::auth_spec::AuthResolution;
use betterlite_mcp::request_headers::raw_headers::RawHeaders;
use secrecy::{ExposeSecret as _, SecretString};

use super::{RESPONSE_HEADER_PREFIX, mask_secret};

const INBOUND_AUTH_HEADERS: [&str; 3] = ["x-litellm-api-key", "authorization", "x-mcp-auth"];
const OAUTH2_TOKEN_HEADER: &str = "Authorization";
const MISSING_VALUE: &str = "(none)";
const MISSING_URL: &str = "(unknown)";
const SAME_AS_LITELLM_KEY: &str = " (SAME_AS_LITELLM_KEY - likely misconfigured)";

#[derive(Debug, Clone, Copy)]
pub struct DebugServer<'server> {
    pub url: Option<&'server str>,
    pub auth_type: Option<&'server str>,
}

#[derive(Debug)]
pub struct DebugHeaderInputs<'headers> {
    pub inbound_headers: &'headers RawHeaders,
    pub oauth2_headers: Option<&'headers BTreeMap<String, SecretString>>,
    pub litellm_api_key: Option<&'headers SecretString>,
    pub auth_resolution: AuthResolution,
    pub server_url: Option<&'headers str>,
    pub server_auth_type: Option<&'headers str>,
}

#[must_use]
pub fn build_debug_headers(inputs: &DebugHeaderInputs<'_>) -> Vec<(String, String)> {
    let mut headers = Vec::with_capacity(5);
    headers.push((
        format!("{RESPONSE_HEADER_PREFIX}-inbound-auth"),
        inbound_auth_summary(inputs.inbound_headers),
    ));
    headers.push((
        format!("{RESPONSE_HEADER_PREFIX}-oauth2-token"),
        oauth2_token_summary(inputs),
    ));
    headers.push((
        format!("{RESPONSE_HEADER_PREFIX}-auth-resolution"),
        inputs.auth_resolution.to_string(),
    ));
    headers.push((
        format!("{RESPONSE_HEADER_PREFIX}-outbound-url"),
        inputs
            .server_url
            .filter(|url| !url.is_empty())
            .unwrap_or(MISSING_URL)
            .to_owned(),
    ));
    headers.push((
        format!("{RESPONSE_HEADER_PREFIX}-server-auth-type"),
        inputs
            .server_auth_type
            .filter(|auth_type| !auth_type.is_empty())
            .unwrap_or(MISSING_VALUE)
            .to_owned(),
    ));
    headers
}

fn inbound_auth_summary(headers: &RawHeaders) -> String {
    let parts: Vec<String> = INBOUND_AUTH_HEADERS
        .iter()
        .filter_map(|name| {
            headers
                .lookup(name)
                .map(|value| format!("{name}={}", mask_secret(Some(value))))
        })
        .collect();
    if parts.is_empty() {
        MISSING_VALUE.to_owned()
    } else {
        parts.join("; ")
    }
}

fn oauth2_token_summary(inputs: &DebugHeaderInputs<'_>) -> String {
    let oauth_token = inputs
        .oauth2_headers
        .and_then(|headers| headers.get(OAUTH2_TOKEN_HEADER))
        .map(SecretString::expose_secret);
    let api_key = inputs.litellm_api_key.map(SecretString::expose_secret);
    let token_credential =
        oauth_token.map(|token| token.strip_prefix("Bearer ").unwrap_or(token).trim());
    let key_credential = api_key.map(|key| key.strip_prefix("Bearer ").unwrap_or(key).trim());
    match (oauth_token, api_key) {
        (Some(token), Some(key))
            if !token.is_empty() && !key.is_empty() && token_credential == key_credential =>
        {
            format!("{}{SAME_AS_LITELLM_KEY}", mask_secret(Some(token)))
        }
        (token, _) => mask_secret(token),
    }
}
