use axum::body::Body;
use axum::http::{Response, StatusCode, header};
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};

use super::SsoReturnTargetError;
use super::cookie::{delete_return_to_cookie, session_token_cookie};
use super::login_code::{
    LOGIN_CODE_PREFIX, LOGIN_CODE_TTL_SECONDS, LoginCode, LoginCodeCache, LoginCodeCaches,
    generate_login_code,
};
use super::origin::validate_return_to;
use super::return_path::is_same_origin_return_path;
use crate::sso_pkce::VerifierEntropy;

// Starlette's RedirectResponse: Python quote with this safe set, plus the unreserved characters
const LOCATION_SAFE: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'!')
    .remove(b'$')
    .remove(b'&')
    .remove(b'\'')
    .remove(b'(')
    .remove(b')')
    .remove(b'*')
    .remove(b'+')
    .remove(b',')
    .remove(b'-')
    .remove(b'.')
    .remove(b'/')
    .remove(b':')
    .remove(b';')
    .remove(b'=')
    .remove(b'?')
    .remove(b'@')
    .remove(b'[')
    .remove(b']')
    .remove(b'_')
    .remove(b'~')
    .remove(b'%')
    .remove(b'#');

pub struct ReturnToRedirect<'request, Redis, User, Entropy> {
    pub now_unix: i64,
    pub return_to: Option<&'request str>,
    pub jwt_token: &'request str,
    pub control_plane_url: Option<&'request str>,
    // axum's Body is not Sync, so a request reference is not Send; the caller resolves the Secure flag instead
    pub secure: bool,
    pub caches: LoginCodeCaches<'request, Redis, User>,
    pub entropy: &'request Entropy,
}

// _sso_return_to_redirect: a relative target redirects with the session cookie, a control-plane one behind a login code
pub async fn sso_return_to_redirect<Redis, User, Entropy>(
    input: &ReturnToRedirect<'_, Redis, User, Entropy>,
) -> Result<Option<Response<Body>>, SsoReturnTargetError>
where
    Redis: LoginCodeCache,
    User: LoginCodeCache,
    Entropy: VerifierEntropy,
{
    let Some(return_to) = input.return_to else {
        return Ok(None);
    };
    if is_same_origin_return_path(return_to) {
        let token = session_token_cookie(input.jwt_token, input.secure)?;
        let cookies = [token, delete_return_to_cookie(input.now_unix)];
        return redirect_response(return_to, &cookies).map(Some);
    }
    if !validate_return_to(return_to, input.control_plane_url)? {
        return Ok(None);
    }
    let code = generate_login_code(input.entropy)?;
    let stored = LoginCode {
        token: input.jwt_token,
        redirect_url: return_to,
    };
    let cache_key = format!("{LOGIN_CODE_PREFIX}{code}");
    input
        .caches
        .store_login_code(&cache_key, &stored, LOGIN_CODE_TTL_SECONDS)
        .await?;
    tracing::info!("Cross-origin SSO: redirecting to control plane with login code");
    let separator = if return_to.contains('?') { '&' } else { '?' };
    let location = format!("{return_to}{separator}login=success&code={code}");
    let cookies = [delete_return_to_cookie(input.now_unix)];
    redirect_response(&location, &cookies).map(Some)
}

fn redirect_response(
    location: &str,
    cookies: &[String],
) -> Result<Response<Body>, SsoReturnTargetError> {
    let encoded = utf8_percent_encode(location, LOCATION_SAFE).to_string();
    let mut builder = Response::builder()
        .status(StatusCode::SEE_OTHER)
        .header(header::LOCATION, encoded.as_str());
    for header_value in cookies {
        builder = builder.header(header::SET_COOKIE, header_value.as_str());
    }
    builder
        .body(Body::empty())
        .map_err(SsoReturnTargetError::Response)
}
