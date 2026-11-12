use std::collections::HashMap;
use std::fmt::Display;
use std::iter::Peekable;
use std::str;
use std::str::Bytes;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE;
use chrono::{DateTime, Datelike as _, NaiveDate, NaiveDateTime, NaiveTime, TimeDelta};
use serde_json::value::RawValue;

use crate::database_url_policy::url_codec::parse_qsl;

pub const MAX_TOKEN_BYTES: usize = 0x0010_0000;
pub const MAX_CLAIMS_DEPTH: usize = 64;

const MAX_YEAR: i32 = 9999;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TokenAuthKind {
    Rds,
    Azure,
}

#[derive(Debug, thiserror::Error)]
pub enum ExpiryError {
    #[error("the token expiration is outside the representable timestamp range")]
    TimestampOutOfRange,
    #[error("the X-Amz-Date value is not a UTC timestamp")]
    MalformedDate,
    #[error("the X-Amz-Expires value is not an integer")]
    MalformedSeconds,
    #[error("the expiration is out of range")]
    OutOfRange,
}

pub fn parse_token_expiration(
    kind: TokenAuthKind,
    token: &str,
) -> Result<Option<NaiveDateTime>, ExpiryError> {
    if token.len() > MAX_TOKEN_BYTES {
        debug_unreadable(kind, &"token exceeds the 1048576 byte limit");
        return Ok(None);
    }
    match kind {
        TokenAuthKind::Rds => Ok(parse_rds_expiration(token)),
        TokenAuthKind::Azure => parse_entra_expiration(token),
    }
}

pub fn decode_database_token_expiry(
    bytes: &[u8],
    kind: TokenAuthKind,
) -> Result<Option<NaiveDateTime>, ExpiryError> {
    if bytes.len() > MAX_TOKEN_BYTES {
        debug_unreadable(kind, &"token exceeds the 1048576 byte limit");
        return Ok(None);
    }
    let Ok(token) = str::from_utf8(bytes) else {
        debug_unreadable(kind, &"token is not valid UTF-8");
        return Ok(None);
    };
    parse_token_expiration(kind, token)
}

fn debug_unreadable(kind: TokenAuthKind, reason: &dyn Display) {
    match kind {
        TokenAuthKind::Rds => {
            tracing::debug!("Failed to parse RDS IAM token expiration: {reason}");
        }
        TokenAuthKind::Azure => {
            tracing::debug!("Failed to parse Azure Entra token expiration: {reason}");
        }
    }
}

fn parse_rds_expiration(token: &str) -> Option<NaiveDateTime> {
    let (_, query) = token.split_once('?')?;
    let params = parse_qsl(query, false);
    let date = first_value(&params, "X-Amz-Date")?;
    let expires = first_value(&params, "X-Amz-Expires")?;
    match rds_expiration(date, expires) {
        Ok(expiration) => Some(expiration),
        Err(reason) => {
            debug_unreadable(TokenAuthKind::Rds, &reason);
            None
        }
    }
}

fn first_value<'params>(params: &'params [(String, String)], name: &str) -> Option<&'params str> {
    params
        .iter()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.as_str())
}

fn rds_expiration(date: &str, expires: &str) -> Result<NaiveDateTime, ExpiryError> {
    let created = rds_date(date).ok_or(ExpiryError::MalformedDate)?;
    let seconds = expires
        .trim()
        .parse::<i64>()
        .map_err(|_error| ExpiryError::MalformedSeconds)?;
    let delta = TimeDelta::try_seconds(seconds).ok_or(ExpiryError::OutOfRange)?;
    let expiration = created
        .checked_add_signed(delta)
        .ok_or(ExpiryError::OutOfRange)?;
    if !(1_i32..=MAX_YEAR).contains(&expiration.year()) {
        return Err(ExpiryError::OutOfRange);
    }
    Ok(expiration)
}

// Mirrors CPython's strptime for "%Y%m%dT%H%M%SZ": a fixed-width year, 1-2 digit fields and case-insensitive literals
fn rds_date(text: &str) -> Option<NaiveDateTime> {
    let mut bytes = text.bytes().peekable();
    let parsed_year = take_year(&mut bytes)?;
    if parsed_year == 0 {
        return None;
    }
    let month = take_field(&mut bytes, 1, 12)?;
    let day = take_field(&mut bytes, 1, 31)?;
    take_literal(&mut bytes, b'T')?;
    let hour = take_field(&mut bytes, 0, 23)?;
    let minute = take_field(&mut bytes, 0, 59)?;
    let second = take_field(&mut bytes, 0, 61)?;
    take_literal(&mut bytes, b'Z')?;
    if bytes.next().is_some() {
        return None;
    }
    let year = i32::try_from(parsed_year).ok()?;
    let date = NaiveDate::from_ymd_opt(year, month, day)?;
    let time = NaiveTime::from_hms_opt(hour, minute, second)?;
    Some(date.and_time(time))
}

fn take_year(bytes: &mut Peekable<Bytes<'_>>) -> Option<u32> {
    let mut year = 0_u32;
    for _digit in 0_u32..4_u32 {
        let digit = decimal(bytes.next()?)?;
        year = year.checked_mul(10_u32)?.checked_add(digit)?;
    }
    Some(year)
}

fn take_field(
    bytes: &mut Peekable<Bytes<'_>>,
    two_digit_min: u32,
    two_digit_max: u32,
) -> Option<u32> {
    let first_digit = bytes.next().and_then(decimal);
    let second_digit = bytes.peek().copied().and_then(decimal);
    if let (Some(high), Some(low)) = (first_digit, second_digit) {
        let pair = high.checked_mul(10_u32)?.checked_add(low)?;
        if (two_digit_min..=two_digit_max).contains(&pair) {
            let _consumed = bytes.next();
            return Some(pair);
        }
    }
    first_digit
}

fn take_literal(bytes: &mut Peekable<Bytes<'_>>, expected: u8) -> Option<()> {
    match bytes.next()? {
        found if found.eq_ignore_ascii_case(&expected) => Some(()),
        _ => None,
    }
}

fn decimal(byte: u8) -> Option<u32> {
    if byte.is_ascii_digit() {
        u32::from(byte).checked_sub(u32::from(b'0'))
    } else {
        None
    }
}

fn parse_entra_expiration(token: &str) -> Result<Option<NaiveDateTime>, ExpiryError> {
    let segments: Vec<&str> = token.split('.').collect();
    let [_, payload, _] = segments.as_slice() else {
        return Ok(None);
    };
    let Some(claims_bytes) = decode_claims(payload) else {
        debug_unreadable(TokenAuthKind::Azure, &"the token payload is not base64url");
        return Ok(None);
    };
    if claims_depth(&claims_bytes) > MAX_CLAIMS_DEPTH {
        debug_unreadable(
            TokenAuthKind::Azure,
            &"the token claims exceed the depth limit",
        );
        return Ok(None);
    }
    // pydantic keeps the last of duplicate keys and ignores extra claims, so the object is read as raw values by name
    let Ok(mut claims) = serde_json::from_slice::<HashMap<String, Box<RawValue>>>(&claims_bytes)
    else {
        debug_unreadable(TokenAuthKind::Azure, &"the token claims are not readable");
        return Ok(None);
    };
    let Some(exp) = claims.remove("exp") else {
        debug_unreadable(TokenAuthKind::Azure, &"the exp claim is missing");
        return Ok(None);
    };
    let Some(seconds) = parse_integer_exp(exp.get())? else {
        debug_unreadable(TokenAuthKind::Azure, &"the exp claim is not an integer");
        return Ok(None);
    };
    entra_timestamp(seconds)
}

// pydantic's int accepts an integer of any width, so an exp past i64 still propagates the timestamp range error
fn parse_integer_exp(raw: &str) -> Result<Option<i64>, ExpiryError> {
    let digits = raw.strip_prefix('-').unwrap_or(raw);
    if !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return Ok(None);
    }
    raw.parse::<i64>()
        .map(Some)
        .map_err(|_error| ExpiryError::TimestampOutOfRange)
}

fn decode_claims(payload: &str) -> Option<Vec<u8>> {
    let unpadded = payload.trim_end_matches('=');
    let padding = match unpadded.len().rem_euclid(4) {
        0 => "",
        2 => "==",
        3 => "=",
        _ => return None,
    };
    let mut encoded = String::from(unpadded);
    encoded.push_str(padding);
    URL_SAFE.decode(encoded).ok()
}

// only the top-level object's nesting counts, and bytes inside a string literal are never braces
fn claims_depth(bytes: &[u8]) -> usize {
    let mut depth = 0_usize;
    let mut deepest = 0_usize;
    let mut in_string = false;
    let mut escaped = false;
    for &byte in bytes {
        if in_string {
            match byte {
                b'\\' if !escaped => escaped = true,
                b'"' if !escaped => in_string = false,
                _ => escaped = false,
            }
            continue;
        }
        match byte {
            b'"' => in_string = true,
            b'{' | b'[' => {
                depth = depth.saturating_add(1);
                deepest = deepest.max(depth);
            }
            b'}' | b']' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    deepest
}

fn entra_timestamp(seconds: i64) -> Result<Option<NaiveDateTime>, ExpiryError> {
    let Some(moment) = DateTime::from_timestamp(seconds, 0_u32) else {
        return Err(ExpiryError::TimestampOutOfRange);
    };
    let expiration = moment.naive_utc();
    if !(1_i32..=MAX_YEAR).contains(&expiration.year()) {
        return Err(ExpiryError::TimestampOutOfRange);
    }
    Ok(Some(expiration))
}
