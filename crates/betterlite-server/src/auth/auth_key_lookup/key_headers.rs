use axum::http::{HeaderMap, header};
use regex::Regex;
use secrecy::SecretString;

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
        strip_prefix(&header_value).unwrap_or_default()
    } else if name == "x-litellm-api-key" {
        strip_prefix(&header_value).unwrap_or(header_value)
    } else {
        header_value
    };
    Some(SecretString::from(token))
}

fn strip_prefix(value: &str) -> Option<String> {
    for prefix in ["Bearer ", "Basic ", "bearer "] {
        if value.starts_with(prefix) {
            return Some(value.replace(prefix, ""));
        }
    }
    value
        .starts_with("AWS4-HMAC-SHA256")
        .then(|| aws_credential(value))
        .flatten()
}

fn aws_credential(value: &str) -> Option<String> {
    let bearer = Regex::new(r"Credential=Bearer\s+([^/\s,]+)").ok()?;
    let plain = Regex::new(r"Credential=([^/\s,]+)").ok()?;
    let captures = bearer.captures(value).or_else(|| plain.captures(value))?;
    captures.get(1).map(|token| token.as_str().to_owned())
}
