use betterlite_error::proxy_error_types::ProxyErrorTypes;
use subtle::ConstantTimeEq as _;
use url::Url;

use crate::generic_sso_callback::failure::GenericSsoErrorResponse;

use super::CallbackGuardError;

pub(super) fn verify_callback_guards(
    state: Option<&str>,
    cookie: Option<&str>,
    code: Option<&str>,
    client_id: &str,
    token_endpoint: &str,
) -> Result<String, CallbackGuardError> {
    // a verifier implies a nonempty state, so absent or empty values fail the bound pair's compare
    let url_state = state.unwrap_or_default();
    let cookie_state = cookie.unwrap_or_default();
    if !bool::from(url_state.as_bytes().ct_eq(cookie_state.as_bytes())) {
        return Err(CallbackGuardError::State);
    }
    let Some(authorization_code) = code.filter(|value| !value.is_empty()) else {
        return Err(CallbackGuardError::Code);
    };
    if client_id.is_empty() {
        return Err(CallbackGuardError::ClientId);
    }
    if token_endpoint.is_empty() {
        return Err(CallbackGuardError::TokenEndpoint);
    }
    Ok(authorization_code.to_owned())
}

pub(super) fn guard_response(status: u16, param: &'static str) -> GenericSsoErrorResponse {
    GenericSsoErrorResponse {
        status,
        error_type: ProxyErrorTypes::AuthError,
        param: Some(param.to_owned()),
        code: status.to_string(),
    }
}

// Starlette's QueryParams.get keeps the last value a key carries
pub(super) fn last_query_value(url: &str, name: &str) -> Option<String> {
    let Ok(parsed) = Url::parse(url) else {
        return None;
    };
    parsed
        .query_pairs()
        .filter(|(key, _)| key == name)
        .map(|(_, value)| value.into_owned())
        .last()
}
