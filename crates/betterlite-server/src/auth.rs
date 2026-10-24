use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, Request, header};
use axum::middleware::Next;
use axum::response::Response;
use secrecy::{ExposeSecret as _, SecretString};
use subtle::ConstantTimeEq as _;

use crate::error_body::authentication_error;

#[derive(Clone)]
pub(crate) struct AuthState {
    pub(crate) master_key: Option<SecretString>,
}

pub(crate) async fn authenticate(
    State(state): State<AuthState>,
    request: Request<Body>,
    next: Next,
) -> Response {
    if is_public_path(request.uri().path()) {
        return next.run(request).await;
    }
    let presented_key = extract_key(request.headers());
    if is_valid_key(presented_key.as_ref(), state.master_key.as_ref()) {
        return next.run(request).await;
    }
    authentication_error(presented_key.is_none())
}

fn is_public_path(path: &str) -> bool {
    let public = matches!(path, "/health/liveliness" | "/health/liveness");
    public
}

fn extract_key(headers: &HeaderMap) -> Option<SecretString> {
    let authorization = headers.get(header::AUTHORIZATION);
    if let Some(value) = authorization.and_then(|value| value.to_str().ok()) {
        let token = value
            .strip_prefix("Bearer ")
            .map_or_else(|| value.to_owned(), str::to_owned);
        return Some(SecretString::from(token));
    }
    for name in [
        "api-key",
        "x-api-key",
        "x-goog-api-key",
        "ocp-apim-subscription-key",
        "x-litellm-api-key",
    ] {
        if let Some(value) = headers.get(name).and_then(|value| value.to_str().ok()) {
            return Some(SecretString::from(value.to_owned()));
        }
    }
    None
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
