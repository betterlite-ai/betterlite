use std::collections::BTreeMap;
use std::str;

use secrecy::{ExposeSecret as _, SecretString};
use serde_json::value::RawValue;

use super::models::{CliAuthContract, PkceFailure};

#[derive(serde::Deserialize)]
pub struct RegisteredClient {
    pub(super) client_id: String,
}

#[derive(serde::Serialize)]
pub(super) struct ClientRegistration {
    pub(super) client_name: &'static str,
    pub(super) redirect_uris: Vec<String>,
    pub(super) grant_types: [&'static str; 2],
    pub(super) response_types: [&'static str; 1],
    pub(super) token_endpoint_auth_method: &'static str,
}

#[derive(Debug)]
pub struct TokenResponse {
    pub(super) access_token: SecretString,
    pub(super) expires_in: i64,
    pub(super) refresh_token: SecretString,
    pub(super) user_id: Option<String>,
    pub(super) team_id: Option<String>,
}

#[derive(serde::Deserialize)]
struct RawTokenResponse {
    access_token: SecretString,
    expires_in: Box<RawValue>,
    refresh_token: SecretString,
    user_id: Option<String>,
    team_id: Option<String>,
}

// LiteLLM's _TokenResponse.expires_in is a plain Pydantic int field. In non-strict (lax) mode that also accepts an integral JSON float (30.0) or a numeric string, whitespace-padded or spelled out with JSON unicode escapes, not only a JSON integer; this reads the field's raw JSON text, decodes it as a JSON string when it is one, and mirrors that coercion while still rejecting non-numeric text, fractional values and anything that overflows i64. No binary float is ever constructed.
fn parse_expires_in(raw: &RawValue) -> Option<i64> {
    let text = raw.get();
    if let Ok(decoded) = serde_json::from_str::<String>(text) {
        return parse_integral_decimal(decoded.trim());
    }
    parse_integral_decimal(text.trim())
}

// Accepts an optional sign, a run of digits, and an optional ".digits" fraction that must be all zero to stay integral; the sign and digit-count checks reject anything that would overflow or truncate i64.
fn parse_integral_decimal(text: &str) -> Option<i64> {
    let (sign, unsigned) = text.strip_prefix('-').map_or_else(
        || (1_i64, text.strip_prefix('+').unwrap_or(text)),
        |rest| (-1_i64, rest),
    );
    let (whole, fraction) = unsigned.split_once('.').unwrap_or((unsigned, ""));
    if whole.is_empty() || !whole.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    if !fraction.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    if fraction.bytes().any(|byte| byte != b'0') {
        return None;
    }
    whole.parse::<i64>().ok()?.checked_mul(sign)
}

// Mirrors LiteLLM's _error_detail: the response body parses into an arbitrary JSON object, and the first of error_description, error, detail that is present and truthy under Python's truthiness rules wins, so an empty string or null falls through to the next field instead of winning as a present-but-blank value.
#[must_use]
pub fn error_detail(body: &[u8]) -> String {
    let detail = serde_json::from_slice::<ErrorBody>(body)
        .ok()
        .and_then(|parsed| {
            [parsed.error_description, parsed.error, parsed.detail]
                .into_iter()
                .flatten()
                .find(|value| is_truthy(value))
                .map(|value| {
                    serde_json::from_str::<String>(value.get())
                        .unwrap_or_else(|_| value.get().to_owned())
                })
        })
        .unwrap_or_else(|| response_text(body));
    detail.chars().take(200).collect()
}

// Every RawValue here is guaranteed syntactically valid JSON, so dispatching on its first byte covers every possible token: a quoted string, true/false/null, an object, an array, or a number. No field is ever parsed through a binary float.
fn is_truthy(raw: &RawValue) -> bool {
    let text = raw.get().trim();
    if let Ok(decoded) = serde_json::from_str::<String>(text) {
        return !decoded.is_empty();
    }
    match text {
        "true" => return true,
        "false" | "null" => return false,
        _ => {}
    }
    if text.starts_with('{') {
        return serde_json::from_str::<BTreeMap<String, Box<RawValue>>>(text)
            .is_ok_and(|fields| !fields.is_empty());
    }
    if text.starts_with('[') {
        return serde_json::from_str::<Vec<Box<RawValue>>>(text)
            .is_ok_and(|items| !items.is_empty());
    }
    is_nonzero_number(text)
}

// A JSON number's exponent only rescales its mantissa, so it never turns a zero mantissa nonzero or a nonzero mantissa zero: truthiness only needs to look at the mantissa's digits, never at a parsed float value.
fn is_nonzero_number(text: &str) -> bool {
    let mantissa = text.split(['e', 'E']).next().unwrap_or(text);
    mantissa
        .bytes()
        .any(|byte| byte.is_ascii_digit() && byte != b'0')
}

fn response_text(body: &[u8]) -> String {
    // requests falls back to Latin-1 for an undeclared charset, so an invalid byte still decodes like LiteLLM's.
    str::from_utf8(body).map_or_else(
        |_source| body.iter().map(|&byte| char::from(byte)).collect(),
        str::to_owned,
    )
}

#[derive(serde::Deserialize)]
struct ErrorBody {
    error_description: Option<Box<RawValue>>,
    error: Option<Box<RawValue>>,
    detail: Option<Box<RawValue>>,
}

pub fn parse_token(body: &[u8]) -> Result<TokenResponse, PkceFailure> {
    let raw: RawTokenResponse =
        serde_json::from_slice(body).map_err(|source| PkceFailure::TokenResponse { source })?;
    let invalid = || PkceFailure::Protocol("invalid token response fields".to_owned());
    let expires_in = parse_expires_in(&raw.expires_in).ok_or_else(invalid)?;
    if raw.access_token.expose_secret().is_empty()
        || raw.refresh_token.expose_secret().is_empty()
        || expires_in <= 0
    {
        return Err(invalid());
    }
    Ok(TokenResponse {
        access_token: raw.access_token,
        expires_in,
        refresh_token: raw.refresh_token,
        user_id: raw.user_id,
        team_id: raw.team_id,
    })
}

// A macro, not two near-identical functions, carries the one difference between the response bodies: their target type and failure variant.
macro_rules! parse_body_fn {
    ($name:ident, $return_ty:ty, $variant:ident) => {
        pub fn $name(body: &[u8]) -> Result<$return_ty, PkceFailure> {
            serde_json::from_slice(body).map_err(|source| PkceFailure::$variant { source })
        }
    };
}

parse_body_fn!(
    parse_registered_client,
    RegisteredClient,
    RegistrationResponse
);
parse_body_fn!(parse_contract, CliAuthContract, DiscoveryDocument);

pub(super) fn refuse_redirect(
    name: &str,
    response: &super::HttpResponse,
) -> Result<(), PkceFailure> {
    if (300..400).contains(&response.status) {
        let location = response
            .headers
            .iter()
            .find(|(header, _)| header.eq_ignore_ascii_case("location"))
            .map_or("another address", |(_, value)| value.as_str());
        return Err(PkceFailure::Protocol(format!(
            "{name} redirected to {location}; refusing to follow it"
        )));
    }
    Ok(())
}
