use chrono::{DateTime, Utc};

use super::cookie::session_token_cookie;
use super::decode_session_cookie::{ClaimText, SessionCookie, decode_session_cookie};

// The dashboard user a signed ``token`` cookie names, with the session key that logout can revoke.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionIdentity {
    pub user_id: String,
    pub session_key: Option<String>,
}

// LiteLLM's _session_identity_from_cookie: the master key signs UI cookies, the type claim keeps MCP
// tokens out, and login_method separates UI tokens from every other master-key token.
#[must_use]
pub fn session_identity_from_cookie(
    cookie_header: Option<&str>,
    master_key: &str,
    now: DateTime<Utc>,
) -> Option<SessionIdentity> {
    if master_key.is_empty() {
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
    let session_key = match &claims.session_key {
        ClaimText::Text(key) if !key.is_empty() => Some(key.clone()),
        ClaimText::Absent | ClaimText::Text(_) | ClaimText::Other => None,
    };
    Some(SessionIdentity {
        user_id: user_id.clone(),
        session_key,
    })
}
