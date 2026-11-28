use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_config::sections::oauth_tokens::Settings;
use http::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue};
use secrecy::{ExposeSecret as _, SecretString};
use thiserror::Error;

pub const FORM_CONTENT_TYPE: &str = "application/x-www-form-urlencoded";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EndpointMode {
    Introspection,
    TokenInfo,
}

#[derive(Debug)]
pub struct PreparedIntrospection {
    pub headers: HeaderMap,
    pub form: Vec<(String, String)>,
}

#[derive(Debug)]
pub struct PreparedTokenInfo {
    pub headers: HeaderMap,
}

#[derive(Debug, Error)]
pub enum PrepareError {
    #[error("the Authorization header cannot carry the token")]
    Authorization,
}

// _is_introspection_endpoint: case-insensitive "introspect" and both credentials present, an empty one included
#[must_use]
pub fn endpoint_mode(settings: &Settings) -> EndpointMode {
    let introspection = settings
        .token_info_endpoint
        .as_deref()
        .is_some_and(|endpoint| {
            endpoint.to_lowercase().contains("introspect")
                && settings.client_id.is_some()
                && settings.client_secret.is_some()
        });
    if introspection {
        EndpointMode::Introspection
    } else {
        EndpointMode::TokenInfo
    }
}

// _prepare_introspection_request: Basic auth only when both credential strings are truthy, else a public client id
pub fn prepare_introspection_request(
    token: &SecretString,
    client_id: Option<&str>,
    client_secret: Option<&SecretString>,
) -> Result<PreparedIntrospection, PrepareError> {
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static(FORM_CONTENT_TYPE));
    let mut form = vec![("token".to_owned(), token.expose_secret().to_owned())];
    let public_client_id = client_id.filter(|value| !value.is_empty());
    let configured_secret = client_secret.filter(|secret| !secret.expose_secret().is_empty());
    match (public_client_id, configured_secret) {
        (Some(id), Some(secret)) => {
            let credentials = STANDARD.encode(format!("{id}:{}", secret.expose_secret()));
            let value = HeaderValue::from_str(&format!("Basic {credentials}"))
                .map_err(|_invalid| PrepareError::Authorization)?;
            headers.insert(AUTHORIZATION, value);
        }
        (Some(id), None) => form.push(("client_id".to_owned(), id.to_owned())),
        (None, _) => {}
    }
    Ok(PreparedIntrospection { headers, form })
}

// _prepare_token_info_request: a Bearer header and a JSON content type, with no query or form parameters
pub fn prepare_token_info_request(token: &SecretString) -> Result<PreparedTokenInfo, PrepareError> {
    let mut headers = HeaderMap::new();
    let bearer = format!("Bearer {}", token.expose_secret());
    let value = HeaderValue::from_str(&bearer).map_err(|_invalid| PrepareError::Authorization)?;
    headers.insert(AUTHORIZATION, value);
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    Ok(PreparedTokenInfo { headers })
}
