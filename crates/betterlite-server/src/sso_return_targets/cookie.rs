use std::net::SocketAddr;

use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{Request, Response, header};
use betterlite_auth::trusted_network::request::{NetworkRequest, is_request_https};
use betterlite_auth::trusted_network::settings::TrustedNetworkSettings;
use chrono::{DateTime, Utc};

use super::SsoReturnTargetError;
use super::origin::validate_return_to;
use super::return_path::is_same_origin_return_path;

pub const TOKEN_COOKIE: &str = "token";
pub const RETURN_TO_COOKIE: &str = "litellm_cp_return_to";
pub const RETURN_TO_MAX_AGE_SECONDS: i64 = 600;
const X_FORWARDED_PROTO: &str = "x-forwarded-proto";

// IPAddressUtils.is_request_https: PROXY_BASE_URL, then a trusted X-Forwarded-Proto, then the request scheme
#[must_use]
pub fn is_secure_request(request: &Request<Body>, settings: &TrustedNetworkSettings) -> bool {
    let peer = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|info| info.0.ip().to_string());
    let forwarded_proto = request.headers().get(X_FORWARDED_PROTO).map(|value| {
        value
            .as_bytes()
            .iter()
            .copied()
            .map(char::from)
            .collect::<String>()
    });
    let network = NetworkRequest {
        peer: peer.as_deref(),
        x_forwarded_for: None,
        x_forwarded_proto: forwarded_proto.as_deref(),
        host: None,
        scheme: request.uri().scheme_str().unwrap_or_default(),
    };
    is_request_https(&network, settings)
}

// set_session_token_cookie: HttpOnly stays off because the dashboard reads this cookie through document.cookie
pub fn set_session_token_cookie(
    response: &mut Response<Body>,
    request: &Request<Body>,
    settings: &TrustedNetworkSettings,
    jwt_token: &str,
) -> Result<(), SsoReturnTargetError> {
    append_cookie(
        response,
        &session_token_cookie(jwt_token, is_secure_request(request, settings))?,
    )
}

#[must_use]
pub fn session_token_cookie(jwt_token: &str, secure: bool) -> Result<String, SsoReturnTargetError> {
    let coded = cookie_value(jwt_token).ok_or(SsoReturnTargetError::CookieValue)?;
    let mut cookie = format!("{TOKEN_COOKIE}={coded}; Path=/; SameSite=lax");
    if secure {
        cookie.push_str("; Secure");
    }
    Ok(cookie)
}

// _persist_return_to_cookie: best effort, so only the origin mismatch is swallowed and sign-in always renders
pub fn persist_return_to_cookie(
    response: &mut Response<Body>,
    return_to: Option<&str>,
    control_plane_url: Option<&str>,
    request: &Request<Body>,
    settings: &TrustedNetworkSettings,
) -> Result<(), SsoReturnTargetError> {
    let Some(target) = return_to else {
        return Ok(());
    };
    let safe = if is_same_origin_return_path(target) {
        true
    } else {
        match validate_return_to(target, control_plane_url) {
            Ok(matched) => matched,
            Err(SsoReturnTargetError::OriginMismatch) => false,
            Err(error) => return Err(error),
        }
    };
    if !safe {
        return Ok(());
    }
    let Some(mut cookie) = return_to_cookie(target) else {
        return Ok(());
    };
    if is_secure_request(request, settings) {
        cookie.push_str("; Secure");
    }
    append_cookie(response, &cookie)
}

#[must_use]
pub fn return_to_cookie(return_to: &str) -> Option<String> {
    let coded = cookie_value(return_to)?;
    Some(format!(
        "{RETURN_TO_COOKIE}={coded}; HttpOnly; Max-Age={RETURN_TO_MAX_AGE_SECONDS}; Path=/; SameSite=lax"
    ))
}

#[must_use]
pub fn delete_return_to_cookie(now_unix: i64) -> String {
    format!(
        "{RETURN_TO_COOKIE}=\"\"; expires={}; Max-Age=0; Path=/; SameSite=lax",
        http_date(now_unix)
    )
}

fn append_cookie(response: &mut Response<Body>, cookie: &str) -> Result<(), SsoReturnTargetError> {
    let value = header::HeaderValue::from_str(cookie).map_err(SsoReturnTargetError::Header)?;
    response.headers_mut().append(header::SET_COOKIE, value);
    Ok(())
}

// http.cookies.SimpleCookie's value encoding: an illegal value is quoted and its illegal letters escaped
fn cookie_value(value: &str) -> Option<String> {
    if value.chars().any(char::is_ascii_control) {
        return None;
    }
    if !value.is_empty() && value.chars().all(is_legal_value_letter) {
        return Some(value.to_owned());
    }
    let mut quoted = String::with_capacity(value.len());
    quoted.push('"');
    for letter in value.chars() {
        push_quoted_letter(&mut quoted, letter);
    }
    quoted.push('"');
    Some(quoted)
}

fn push_quoted_letter(quoted: &mut String, letter: char) {
    if letter == '"' {
        quoted.push_str("\\\"");
    } else if letter == '\\' {
        quoted.push_str("\\\\");
    } else if is_unescaped_value_letter(letter) || u32::from(letter) > 255 {
        quoted.push(letter);
    } else {
        let code = u32::from(letter);
        let escaped = format!("\\{code:03o}");
        quoted.push_str(&escaped);
    }
}

const fn is_legal_value_letter(letter: char) -> bool {
    letter.is_ascii_alphanumeric()
        || matches!(
            letter,
            '!' | '#'
                | '$'
                | '%'
                | '&'
                | '\''
                | '*'
                | '+'
                | '-'
                | '.'
                | '^'
                | '_'
                | '`'
                | '|'
                | '~'
                | ':'
        )
}

const fn is_unescaped_value_letter(letter: char) -> bool {
    is_legal_value_letter(letter)
        || matches!(
            letter,
            ' ' | '(' | ')' | '/' | '<' | '=' | '>' | '?' | '@' | '[' | ']' | '{' | '}'
        )
}

// http.cookies._getdate: the GMT timestamp of that moment, with the abbreviation tables of the stdlib
fn http_date(now_unix: i64) -> String {
    DateTime::from_timestamp(now_unix, 0)
        .unwrap_or(DateTime::<Utc>::UNIX_EPOCH)
        .format("%a, %d %b %Y %H:%M:%S GMT")
        .to_string()
}
