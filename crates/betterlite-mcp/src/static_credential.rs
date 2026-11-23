mod tests;

use std::collections::BTreeSet;

use base64::Engine as _;
use base64::alphabet;
use base64::engine::general_purpose::{GeneralPurpose, GeneralPurposeConfig};

use crate::credential_contracts::errors::CredError;
use crate::credentials::{McpAuthType, strip_auth_scheme};

// CPython's strict decoder ignores the unused bits of a final group, so a hand-padded value still decodes
const BASE64: GeneralPurpose = GeneralPurpose::new(
    &alphabet::STANDARD,
    GeneralPurposeConfig::new().with_decode_allow_trailing_bits(true),
);

const API_KEY_HEADER: &str = "X-API-Key";
const AUTHORIZATION_HEADER: &str = "Authorization";
const SCHEME_WORDS: [&str; 4] = ["bearer", "basic", "token", "apikey"];
const API_KEY_SCHEMES: [&str; 3] = ["bearer", "token", "apikey"];

pub fn validate_static_credential(
    auth_type: McpAuthType,
    headers: &BTreeMap<String, String>,
    upstream_token_header: Option<&str>,
    static_header_names: &[String],
) -> Result<(), CredError> {
    let (default_header, mode) = match auth_type {
        McpAuthType::ApiKey => (API_KEY_HEADER, "api_key"),
        McpAuthType::BearerToken => (AUTHORIZATION_HEADER, "bearer_token"),
        McpAuthType::Basic => (AUTHORIZATION_HEADER, "basic"),
        McpAuthType::Authorization => (AUTHORIZATION_HEADER, "authorization"),
        McpAuthType::Token => (AUTHORIZATION_HEADER, "token"),
        McpAuthType::None | McpAuthType::Oauth2 | McpAuthType::Oauth2TokenExchange => return Ok(()),
    };
    let mut slots = BTreeSet::new();
    slots.insert(default_header.to_lowercase());
    slots.insert(AUTHORIZATION_HEADER.to_lowercase());
    slots.insert(
        upstream_token_header
            .unwrap_or(default_header)
            .to_lowercase(),
    );
    if auth_type == McpAuthType::ApiKey {
        for name in static_header_names {
            slots.insert(name.to_lowercase());
        }
    }
    for (name, value) in headers {
        let slot = name.to_lowercase();
        if slots.contains(&slot) && usable_credential_value(auth_type, &slot, value.trim()) {
            return Ok(());
        }
    }
    Err(CredError::Misconfigured(format!(
        "{mode} requires a usable upstream credential"
    )))
}

fn usable_credential_value(auth_type: McpAuthType, name: &str, value: &str) -> bool {
    if value.is_empty() {
        return false;
    }
    if auth_type == McpAuthType::ApiKey && name != "authorization" {
        return true;
    }
    if SCHEME_WORDS
        .iter()
        .any(|word| value.eq_ignore_ascii_case(word))
    {
        return false;
    }
    if auth_type == McpAuthType::ApiKey {
        let scheme = value
            .split_once(char::is_whitespace)
            .map_or(value, |parts| parts.0);
        if API_KEY_SCHEMES
            .iter()
            .any(|word| scheme.eq_ignore_ascii_case(word))
        {
            return !strip_auth_scheme(value, scheme)
                .trim()
                .eq_ignore_ascii_case(scheme);
        }
    }
    match auth_type {
        McpAuthType::BearerToken => has_credential_after(value, "Bearer"),
        McpAuthType::Token => has_credential_after(value, "token"),
        McpAuthType::Basic => basic_carries_credentials(value),
        McpAuthType::ApiKey
        | McpAuthType::Authorization
        | McpAuthType::None
        | McpAuthType::Oauth2
        | McpAuthType::Oauth2TokenExchange => true,
    }
}

fn has_credential_after(value: &str, scheme: &str) -> bool {
    let stripped = strip_auth_scheme(value, scheme);
    let credential = stripped.trim();
    !credential.is_empty() && !credential.eq_ignore_ascii_case(scheme)
}

fn basic_carries_credentials(value: &str) -> bool {
    let Some((scheme, encoded)) = value.split_once(char::is_whitespace) else {
        return false;
    };
    if !scheme.eq_ignore_ascii_case("basic") {
        return false;
    }
    let Ok(decoded) = BASE64.decode(encoded.trim()) else {
        return false;
    };
    decoded.contains(&b':')
}
