use chrono::{DateTime, Utc};
use secrecy::{ExposeSecret as _, SecretString};

use super::cookie::session_token_cookie;
use super::decode_session_cookie::{ClaimText, SessionCookie, decode_session_cookie};

// The dashboard user a signed ``token`` cookie names, with the session key that logout can revoke.
#[derive(Clone, Debug)]
pub struct SessionIdentity {
    pub user_id: String,
    pub session_key: Option<SecretString>,
}

// LiteLLM's _session_identity_from_cookie: the master key signs UI cookies and login_method separates UI tokens.
#[must_use]
pub fn session_identity_from_cookie(
    cookie_header: Option<&str>,
    master_key: &SecretString,
    now: DateTime<Utc>,
) -> Option<SessionIdentity> {
    if master_key.expose_secret().is_empty() {
        return None;
    }
    let token = session_token_cookie(cookie_header)?;
    let cookie = decode_session_cookie(token.as_bytes()).ok()?;
    if !cookie.signature_matches(master_key) || !cookie.claims_are_current(now) {
        return None;
    }
    identity(&cookie)
}

fn identity(cookie: &SessionCookie) -> Option<SessionIdentity> {
    let claims = &cookie.claims;
    if matches!(&claims.token_type, ClaimText::Text(name) if name == "byok_session") {
        return None;
    }
    match &claims.login_method {
        ClaimText::Text(method) if method == "sso" || method == "username_password" => {}
        ClaimText::Absent | ClaimText::Text(_) | ClaimText::Other => return None,
    }
    let ClaimText::Text(user_id) = &claims.user_id else {
        return None;
    };
    if user_id.is_empty() {
        return None;
    }
    Some(SessionIdentity {
        user_id: user_id.clone(),
        session_key: claims.session_key.clone(),
    })
}
