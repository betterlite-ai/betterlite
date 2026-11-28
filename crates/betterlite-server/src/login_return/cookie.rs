use secrecy::{ExposeSecret as _, SecretString};

pub const SESSION_COOKIE_NAME: &str = "token";
pub const RETURN_COOKIE_NAME: &str = "litellm_cp_return_to";

// set_session_token_cookie: no HttpOnly (dashboard JavaScript reads it); Secure comes from the caller.
#[must_use]
pub fn session_cookie_header(token: &SecretString, secure: bool) -> String {
    let mut header = format!("{SESSION_COOKIE_NAME}={}", token.expose_secret());
    header.push_str("; Path=/; SameSite=lax");
    if secure {
        header.push_str("; Secure");
    }
    header
}

// RedirectResponse.delete_cookie defaults: Path=/, Max-Age=0, samesite=lax, no Secure or HttpOnly.
#[must_use]
pub fn clear_return_cookie_header() -> String {
    format!(
        "{RETURN_COOKIE_NAME}=\"\"; expires=Thu, 01 Jan 1970 00:00:00 GMT; Max-Age=0; Path=/; SameSite=lax"
    )
}
