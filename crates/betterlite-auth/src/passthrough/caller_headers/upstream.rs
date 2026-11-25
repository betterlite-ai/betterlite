use http::HeaderMap;
use indexmap::IndexMap;
use secrecy::SecretString;

use crate::resolved_key::auth::UserAPIKeyAuth;

use super::caller_filter::{FilterSettings, caller_headers_without_secrets};
use super::error::CallerHeaderError;
use super::header_sets::{
    ANTHROPIC_CREDENTIAL_HEADERS, ANTHROPIC_NEVER_FORWARDED, BEDROCK_NEVER_FORWARDED,
    VERTEX_CREDENTIAL_HEADERS, VERTEX_NEVER_FORWARDED,
};

#[must_use]
pub fn vertex_credentialless_headers(
    headers: &HeaderMap,
    auth: &UserAPIKeyAuth,
    settings: &FilterSettings<'_>,
) -> Result<IndexMap<String, SecretString>, CallerHeaderError> {
    let forwarded =
        caller_headers_without_secrets(headers, auth, &VERTEX_NEVER_FORWARDED, settings);
    let carries_credential = VERTEX_CREDENTIAL_HEADERS
        .iter()
        .any(|name| forwarded.contains_key(*name));
    if !carries_credential {
        return Err(CallerHeaderError::VertexCredentialMissing);
    }
    Ok(forwarded)
}

#[must_use]
pub fn anthropic_upstream_headers(
    headers: &HeaderMap,
    auth: &UserAPIKeyAuth,
    proxy_auth_header: Option<&IndexMap<String, SecretString>>,
    settings: &FilterSettings<'_>,
) -> Result<IndexMap<String, SecretString>, CallerHeaderError> {
    let mut caller_headers =
        caller_headers_without_secrets(headers, auth, &ANTHROPIC_NEVER_FORWARDED, settings);
    let caller_carries_credential = ANTHROPIC_CREDENTIAL_HEADERS
        .iter()
        .any(|name| caller_headers.contains_key(*name));
    if proxy_auth_header.is_none() && !caller_carries_credential {
        return Err(CallerHeaderError::AnthropicCredentialMissing);
    }
    if let Some(proxy) = proxy_auth_header {
        for (name, value) in proxy {
            caller_headers.insert(name.clone(), value.clone());
        }
    }
    Ok(caller_headers)
}

#[must_use]
pub fn bedrock_runtime_headers(
    headers: &HeaderMap,
    auth: &UserAPIKeyAuth,
    signed_headers: &IndexMap<String, SecretString>,
    settings: &FilterSettings<'_>,
) -> IndexMap<String, SecretString> {
    let mut dropped_names: Vec<String> = BEDROCK_NEVER_FORWARDED
        .iter()
        .map(|name| (*name).to_owned())
        .collect();
    dropped_names.extend(signed_headers.keys().map(|name| name.to_lowercase()));
    let dropped: Vec<&str> = dropped_names.iter().map(String::as_str).collect();
    let mut forwarded = caller_headers_without_secrets(headers, auth, &dropped, settings);
    for (name, value) in signed_headers {
        forwarded.insert(name.clone(), value.clone());
    }
    forwarded
}
