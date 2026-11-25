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

// the credential has to survive the filter, so a header holding the master key or the stored key never counts
fn carries_caller_credential(
    forwarded: &IndexMap<String, SecretString>,
    credential_headers: &[&str],
) -> bool {
    for name in credential_headers {
        if forwarded.contains_key(*name) {
            return true;
        }
    }
    false
}

#[must_use]
pub fn vertex_credentialless_headers(
    headers: &HeaderMap,
    auth: &UserAPIKeyAuth,
    settings: &FilterSettings<'_>,
) -> Result<IndexMap<String, SecretString>, CallerHeaderError> {
    let forwarded =
        caller_headers_without_secrets(headers, auth, &VERTEX_NEVER_FORWARDED, settings);
    if carries_caller_credential(&forwarded, VERTEX_CREDENTIAL_HEADERS) {
        return Ok(forwarded);
    }
    Err(CallerHeaderError::VertexCredentialMissing)
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
    let caller_has_credential =
        carries_caller_credential(&caller_headers, ANTHROPIC_CREDENTIAL_HEADERS);
    if proxy_auth_header.is_none() && !caller_has_credential {
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
