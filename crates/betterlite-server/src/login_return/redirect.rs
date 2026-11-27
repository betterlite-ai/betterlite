use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use betterlite_auth::trusted_network::request::{NetworkRequest, is_request_https};
use betterlite_auth::trusted_network::settings::TrustedNetworkSettings;
use betterlite_config::sections::login_return::Settings as LoginSettings;
use http::StatusCode;
use secrecy::SecretString;
use thiserror::Error;

use crate::login_return::cache::{
    LOGIN_CODE_PREFIX, LOGIN_CODE_TTL_SECONDS, LoginCaches, LoginCode, LoginCodeCache,
    LoginCodeCacheError,
};
use crate::login_return::cookie::{
    clear_return_cookie_header, quote_location, session_cookie_header,
};
use crate::login_return::origin::{ReturnToError, validate_return_to};
use crate::login_return::return_destination::{
    ReturnDestinationError, decode_return_destination, is_same_origin_return_path,
};
use crate::sso_pkce::VerifierEntropy;

const CODE_RANDOM_BYTES: usize = 32;

// no Debug: set_cookies carries the session token
pub struct LoginRedirect {
    pub status: StatusCode,
    pub location: String,
    pub set_cookies: Vec<String>,
}

#[derive(Debug)]
pub struct ResumeRequest<'request> {
    pub destination: Option<&'request [u8]>,
    pub session_token: &'request SecretString,
    pub network: NetworkRequest<'request>,
}

#[derive(Debug, Error)]
pub enum LoginReturnError {
    #[error(transparent)]
    Destination(#[from] ReturnDestinationError),
    #[error(transparent)]
    ReturnTo(#[from] ReturnToError),
    #[error("could not draw random bytes for the login code")]
    Entropy(#[from] getrandom::Error),
    #[error(transparent)]
    Cache(#[from] LoginCodeCacheError),
}

// _sso_return_to_redirect: a relative same-origin path gets the session cookie and the exact path
// back, a control-plane origin gets a single-use code the control plane redeems, and every other
// destination, including an absolute one with no configured control plane, falls through to None.
pub async fn resume_login<Redis, User, Entropy>(
    request: ResumeRequest<'_>,
    settings: &LoginSettings,
    trusted_network: &TrustedNetworkSettings,
    caches: LoginCaches<'_, Redis, User>,
    entropy: &Entropy,
) -> Result<Option<LoginRedirect>, LoginReturnError>
where
    Redis: LoginCodeCache,
    User: LoginCodeCache,
    Entropy: VerifierEntropy,
{
    let Some(destination) = request.destination else {
        return Ok(None);
    };
    let return_to = decode_return_destination(destination)?;
    if is_same_origin_return_path(&return_to) {
        let secure = is_request_https(&request.network, trusted_network);
        return Ok(Some(relative_redirect(
            &return_to,
            request.session_token,
            secure,
        )));
    }
    if !validate_return_to(&return_to, settings.control_plane_url.as_deref())? {
        return Ok(None);
    }
    let code = mint_login_code(entropy)?;
    let stored = LoginCode {
        token: (*request.session_token).clone(),
        redirect_url: return_to.clone(),
    };
    caches
        .store(
            &format!("{LOGIN_CODE_PREFIX}{code}"),
            &stored,
            LOGIN_CODE_TTL_SECONDS,
        )
        .await?;
    // the handoff line names neither the token nor the single-use code
    tracing::info!("Cross-origin SSO: redirecting to control plane with login code");
    Ok(Some(control_plane_redirect(&return_to, &code)))
}

fn relative_redirect(return_to: &str, token: &SecretString, secure: bool) -> LoginRedirect {
    LoginRedirect {
        status: StatusCode::SEE_OTHER,
        location: quote_location(return_to),
        set_cookies: vec![
            session_cookie_header(token, secure),
            clear_return_cookie_header(),
        ],
    }
}

fn control_plane_redirect(return_to: &str, code: &str) -> LoginRedirect {
    let separator = if return_to.contains('?') { '&' } else { '?' };
    let location = format!("{return_to}{separator}login=success&code={code}");
    LoginRedirect {
        status: StatusCode::SEE_OTHER,
        location: quote_location(&location),
        set_cookies: vec![clear_return_cookie_header()],
    }
}

// secrets.token_urlsafe(32): 32 random bytes as unpadded URL-safe base64
fn mint_login_code<Entropy>(entropy: &Entropy) -> Result<String, getrandom::Error>
where
    Entropy: VerifierEntropy,
{
    let mut bytes = [0_u8; CODE_RANDOM_BYTES];
    entropy.fill(&mut bytes)?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}
