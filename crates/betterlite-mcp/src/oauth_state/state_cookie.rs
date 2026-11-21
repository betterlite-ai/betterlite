mod tests;

use crate::oauth_resource::python_url;

pub const STATE_COOKIE_PREFIX: &str = "mcp_oauth_state_";
pub const STATE_COOKIE_TTL_SECONDS: u64 = 600;
pub const STATE_COOKIE_HANDLE_BYTES: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SameSite {
    Strict,
    Lax,
    None,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StateCookie {
    pub name: String,
    pub value: String,
    pub max_age: u64,
    pub path: String,
    pub secure: bool,
    pub http_only: bool,
    pub same_site: SameSite,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClearCookie {
    pub name: String,
    pub path: String,
    pub secure: bool,
    pub http_only: bool,
    pub same_site: SameSite,
}

#[must_use]
pub fn state_cookie_name(relay_state: &str) -> String {
    let mut name = String::from(STATE_COOKIE_PREFIX);
    name.push_str(relay_state);
    name
}

#[must_use]
pub fn cookie_path_and_secure(base_url: &str) -> (String, bool) {
    let Ok(parsed) = python_url::parse(base_url) else {
        return ("/".to_owned(), false);
    };
    let path = if parsed.path.is_empty() {
        "/".to_owned()
    } else {
        parsed.path
    };
    (path, parsed.scheme == "https")
}

#[must_use]
pub fn set_state_cookie(base_url: &str, relay_state: &str, encoded_state: &str) -> StateCookie {
    let (path, secure) = cookie_path_and_secure(base_url);
    StateCookie {
        name: state_cookie_name(relay_state),
        value: encoded_state.to_owned(),
        max_age: STATE_COOKIE_TTL_SECONDS,
        path,
        secure,
        http_only: true,
        same_site: SameSite::Lax,
    }
}

#[must_use]
pub fn resolve_encoded_state(cookies: &dyn Fn(&str) -> Option<String>, state: &str) -> String {
    match cookies(&state_cookie_name(state)) {
        Some(encoded) if !encoded.is_empty() => encoded,
        Some(_empty) | None => state.to_owned(),
    }
}

#[must_use]
pub fn clear_state_cookie(
    base_url: &str,
    cookie_present: bool,
    state: &str,
) -> Option<ClearCookie> {
    if !cookie_present {
        return None;
    }
    let (path, secure) = cookie_path_and_secure(base_url);
    Some(ClearCookie {
        name: state_cookie_name(state),
        path,
        secure,
        http_only: true,
        same_site: SameSite::Lax,
    })
}
