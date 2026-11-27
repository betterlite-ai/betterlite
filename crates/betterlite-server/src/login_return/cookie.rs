use secrecy::{ExposeSecret as _, SecretString};

pub const SESSION_COOKIE_NAME: &str = "token";
pub const RETURN_COOKIE_NAME: &str = "litellm_cp_return_to";

// Location quoting: urllib.parse.quote with safe=":/%#?=@[]!$&'()*+,;" over alphanumerics and "_.-~"
const LOCATION_SAFE: &str = ":/%#?=@[]!$&'()*+,;";

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

// urllib.parse.quote(url, safe=":/%#?=@[]!$&'()*+,;"), the header value Starlette writes for Location
#[must_use]
pub fn quote_location(url: &str) -> String {
    let mut quoted = String::with_capacity(url.len());
    for byte in url.bytes() {
        let safe = byte.is_ascii_alphanumeric()
            || matches!(byte, b'_' | b'.' | b'-' | b'~')
            || LOCATION_SAFE.as_bytes().contains(&byte);
        if safe {
            quoted.push(char::from(byte));
            continue;
        }
        let escaped = format!("%{byte:02X}");
        quoted.push_str(&escaped);
    }
    quoted
}
