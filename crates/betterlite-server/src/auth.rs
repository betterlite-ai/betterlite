use crate::error_body::{AuthenticationFailure, authentication_error};
use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, Request, header};
use axum::middleware::Next;
use axum::response::Response;
use regex::Regex;
use secrecy::{ExposeSecret as _, SecretString};
use subtle::ConstantTimeEq as _;
#[derive(Clone)]
pub(crate) struct AuthState {
    pub(crate) master_key: Option<SecretString>,
}
pub(crate) async fn authenticate(
    State(state): State<AuthState>,
    request: Request<Body>,
    next: Next,
) -> Response {
    if matches!(
        request.uri().path(),
        "/health/liveliness" | "/health/liveness"
    ) {
        return next.run(request).await;
    }
    let Some(master_key) = state.master_key.as_ref() else {
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
        Some(key) if key.expose_secret().is_empty() => AuthenticationFailure::Malformed,
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
        if let Some(value) = headers.get(name)
            && let Some(key) = parse_key_header(name, value.as_bytes())
        {
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
        strip_prefix(&header_value).unwrap_or_else(|| header_value.clone())
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
    if value.starts_with("AWS4-HMAC-SHA256") {
        return aws_credential(value);
    }
    None
}
fn aws_credential(value: &str) -> Option<String> {
    let bearer_pattern = Regex::new(r"Credential=Bearer\s+([^/\s,]+)").ok()?;
    if let Some(token) = bearer_pattern
        .captures(value)
        .and_then(|captures| captures.get(1))
    {
        return Some(token.as_str().to_owned());
    }
    let credential_pattern = Regex::new(r"Credential=([^/\s,]+)").ok()?;
    credential_pattern
        .captures(value)?
        .get(1)
        .map(|token| token.as_str().to_owned())
}
