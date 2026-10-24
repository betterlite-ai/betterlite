use crate::error_body::{AuthenticationFailure, authentication_error};
use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, Request, header};
use axum::middleware::Next;
use axum::response::Response;
use regex::Regex;
use secrecy::{ExposeSecret as _, SecretString};
use std::str;
use subtle::ConstantTimeEq as _;
pub(crate) async fn authenticate(
    State(master_key_state): State<Option<SecretString>>,
    request: Request<Body>,
    next: Next,
) -> Response {
    if matches!(
        request.uri().path(),
        "/health/liveliness" | "/health/liveness" | "/health/readiness"
    ) {
        return next.run(request).await;
    }
    let Some(master_key) = master_key_state.as_ref() else {
        return next.run(request).await;
    };
    let presented_key = extract_key(request.headers());
    if presented_key.as_ref().is_some_and(|presented| {
        bool::from(
            presented
                .expose_secret()
                .as_bytes()
                .ct_eq(master_key.expose_secret().as_bytes()),
        )
    }) {
        return next.run(request).await;
    }
    let failure = match presented_key.as_ref() {
        None => AuthenticationFailure::Missing,
        Some(key) if key.expose_secret().is_empty() => AuthenticationFailure::Missing,
        Some(_) => AuthenticationFailure::Invalid,
    };
    authentication_error(&failure)
}
fn extract_key(headers: &HeaderMap) -> Option<SecretString> {
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
    let header_value = str::from_utf8(header_bytes).ok()?.to_owned();
    if name == "x-litellm-api-key" && !header_value.is_ascii() {
        return None;
    }
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
