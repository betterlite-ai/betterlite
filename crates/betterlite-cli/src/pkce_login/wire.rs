use std::iter::repeat_n;
use std::str;

use secrecy::{ExposeSecret as _, SecretString};
use serde::de::DeserializeOwned;
use serde_json::value::RawValue;

use super::models::PkceFailure;

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
    expires_in: ExpiresIn,
    refresh_token: SecretString,
    user_id: Option<String>,
    team_id: Option<String>,
}

#[derive(serde::Deserialize)]
#[serde(untagged)]
enum ExpiresIn {
    Number(serde_json::Number),
    Text(String),
}

fn parse_expires_in(value: ExpiresIn) -> Option<i64> {
    let text = match value {
        ExpiresIn::Number(number) => number.to_string(),
        ExpiresIn::Text(text) => text,
    };
    parse_integral_number(&text)
}

// Pydantic accepts integral decimal/exponent forms and underscores in numeric strings; integer arithmetic avoids float rounding at the i64 boundary.
fn parse_integral_number(text: &str) -> Option<i64> {
    let normalized_text = text.trim().replace('_', "");
    let (mantissa, exponent) =
        if let Some((value, exponent_text)) = normalized_text.split_once(['e', 'E']) {
            (value, exponent_text.parse::<i32>().ok()?)
        } else {
            (normalized_text.as_str(), 0_i32)
        };
    let (negative, signed_mantissa) = mantissa
        .strip_prefix('-')
        .map_or((false, mantissa), |value| (true, value));
    let unsigned_mantissa = signed_mantissa.strip_prefix('+').unwrap_or(signed_mantissa);
    let (whole, fraction) = unsigned_mantissa
        .split_once('.')
        .unwrap_or((unsigned_mantissa, ""));
    if whole.is_empty() && fraction.is_empty()
        || !whole.bytes().all(|byte| byte.is_ascii_digit())
        || !fraction.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    let digits = format!("{whole}{fraction}");
    let integer_digits = normalize_integer_digits(&digits, whole.len(), exponent)?;
    let magnitude = integer_digits.trim_start_matches('0');
    let signed_digits = if negative {
        format!("-{magnitude}")
    } else {
        magnitude.to_owned()
    };
    signed_digits.parse::<i64>().ok()
}

// i64::MIN and i64::MAX both print as 19 digits, so a decimal position past that can only overflow; rejecting it here, before any zero-padding, keeps the allocation bounded regardless of how large the exponent claims to be.
const MAX_I64_DIGITS: usize = 19;

pub(super) fn normalize_integer_digits(
    digits: &str,
    whole_length: usize,
    exponent: i32,
) -> Option<String> {
    let decimal_position = i64::try_from(whole_length)
        .ok()?
        .checked_add(i64::from(exponent))?;
    let integer_digits = if decimal_position <= 0 {
        if digits.bytes().any(|byte| byte != b'0') {
            return None;
        }
        "0".to_owned()
    } else {
        let position = usize::try_from(decimal_position).ok()?;
        if position < digits.len() {
            let integer = digits.get(..position)?;
            let fractional = digits.get(position..)?;
            if fractional.bytes().any(|byte| byte != b'0') {
                return None;
            }
            integer.to_owned()
        } else {
            if position > MAX_I64_DIGITS {
                return None;
            }
            let mut integer = digits.to_owned();
            let zeros = position.checked_sub(integer.len())?;
            integer.extend(repeat_n('0', zeros));
            integer
        }
    };
    Some(integer_digits)
}

// Mirrors LiteLLM's _error_detail field order, but not its Python `or` chaining: docs/compatibility.md treats that as a runtime habit, so a present field wins even when it is "", 0 or false, and only an absent or explicit null field falls through to the next one.
#[must_use]
pub fn error_detail(body: &[u8]) -> String {
    let detail = serde_json::from_slice::<ErrorBody>(body)
        .ok()
        .and_then(|parsed| {
            [parsed.error_description, parsed.error, parsed.detail]
                .into_iter()
                .flatten()
                .next()
                .map(|value| {
                    serde_json::from_str::<String>(value.get())
                        .unwrap_or_else(|_| value.get().to_owned())
                })
        })
        .unwrap_or_else(|| response_text(body));
    detail.chars().take(200).collect()
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
    let expires_in = parse_expires_in(raw.expires_in).ok_or_else(invalid)?;
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

// Shared by every parser here that deserializes a response body and then rejects one invariant the type itself can't express.
pub(super) fn parse_and_require<T>(
    body: &[u8],
    to_error: impl FnOnce(serde_json::Error) -> PkceFailure,
    invalid: impl FnOnce(&T) -> bool,
    message: &str,
) -> Result<T, PkceFailure>
where
    T: DeserializeOwned,
{
    let value: T = serde_json::from_slice(body).map_err(to_error)?;
    if invalid(&value) {
        return Err(PkceFailure::Protocol(message.to_owned()));
    }
    Ok(value)
}

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
