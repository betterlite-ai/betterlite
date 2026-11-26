use axum::http::{HeaderMap, header};
use secrecy::SecretString;

use crate::auth::key_prefix;

pub(crate) fn extract_key(headers: &HeaderMap) -> Option<SecretString> {
    for name in [
        "x-litellm-api-key",
        header::AUTHORIZATION.as_str(),
        "api-key",
        "x-api-key",
        "x-goog-api-key",
        "ocp-apim-subscription-key",
    ] {
        let Some(value) = headers.get(name) else {
            continue;
        };
        if let Some(key) = parse_key_header(name, value.as_bytes()) {
            return Some(key);
        }
    }
    None
}

pub fn parse_key_header(name: &str, header_bytes: &[u8]) -> Option<SecretString> {
    if header_bytes.is_empty() {
        return None;
    }
    let header_value: String = header_bytes.iter().map(|byte| char::from(*byte)).collect();
    let token = if name == header::AUTHORIZATION.as_str() {
        key_prefix::strip_prefix(&header_value).unwrap_or_default()
    } else if name == "x-litellm-api-key" {
        key_prefix::strip_prefix(&header_value).unwrap_or(header_value)
    } else {
        header_value
    };
    Some(SecretString::from(token))
}
