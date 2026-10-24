use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, Request, header};
use axum::middleware::Next;
use axum::response::Response;
use secrecy::{ExposeSecret as _, SecretString};
use subtle::ConstantTimeEq as _;

use crate::error_body::{AuthenticationFailure, authentication_error};

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
    if state.master_key.is_none() {
        return next.run(request).await;
    }
    let presented_key = extract_key(request.headers());
    if is_valid_key(presented_key.as_ref(), state.master_key.as_ref()) {
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
        if let Some(value) = headers.get(name).and_then(|value| value.to_str().ok()) {
            if name == header::AUTHORIZATION.as_str() && value.is_empty() {
                continue;
            }
            let token = if name == header::AUTHORIZATION.as_str() {
                strip_prefix(value).unwrap_or_default()
            } else if name == "x-litellm-api-key" {
                strip_prefix(value).unwrap_or_else(|| value.to_owned())
            } else {
                value.to_owned()
            };
            return Some(SecretString::from(token));
        }
    }
    None
}

fn strip_prefix(value: &str) -> Option<String> {
    for prefix in ["Bearer ", "Basic ", "bearer "] {
        if let Some(token) = value.strip_prefix(prefix) {
            return Some(token.to_owned());
        }
    }
    if value.starts_with("AWS4-HMAC-SHA256") {
        return aws_credential(value);
    }
    None
}

fn aws_credential(value: &str) -> Option<String> {
    let credential = value.split_once("Credential=")?.1;
    let credential_without_prefix = credential
        .strip_prefix("Bearer ")
        .unwrap_or(credential)
        .split(|character: char| character == '/' || character == ',' || character.is_whitespace())
        .next()
        .filter(|token| !token.is_empty())?;
    Some(credential_without_prefix.to_owned())
}

fn is_valid_key(presented_key: Option<&SecretString>, master_key: Option<&SecretString>) -> bool {
    let (Some(presented), Some(configured)) = (presented_key, master_key) else {
        return false;
    };
    bool::from(
        presented
            .expose_secret()
            .as_bytes()
            .ct_eq(configured.expose_secret().as_bytes()),
    )
}
