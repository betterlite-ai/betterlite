use std::str::{Utf8Error, from_utf8};
use std::time::{SystemTime, UNIX_EPOCH};

use thiserror::Error;

// the parser never allocates past this many input bytes, so a header from a provider cannot size a request
pub const MAX_RETRY_AFTER_BYTES: usize = 8192;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetryAfter {
    Seconds(i64),
    At { unix_millis: i64 },
}

impl RetryAfter {
    // Python's int(retry_date - time.time()) truncates toward zero, which Rust's checked division does too
    #[must_use]
    pub fn seconds_until(self, now_unix_millis: i64) -> i64 {
        match self {
            Self::Seconds(seconds) => seconds,
            Self::At { unix_millis } => unix_millis
                .saturating_sub(now_unix_millis)
                .checked_div(1000)
                .unwrap_or(0),
        }
    }
}

#[derive(Debug, Error)]
pub enum RetryAfterError {
    #[error("the retry-after value is {length} bytes, over the 8192-byte limit")]
    TooLarge { length: usize },
    #[error("the retry-after value is not UTF-8")]
    NotUtf8 {
        #[source]
        source: Utf8Error,
    },
    #[error("the retry-after value is not an HTTP date")]
    NotDate {
        #[source]
        source: httpdate::Error,
    },
}

pub fn parse_retry_after(bytes: &[u8]) -> Result<RetryAfter, RetryAfterError> {
    if bytes.len() > MAX_RETRY_AFTER_BYTES {
        return Err(RetryAfterError::TooLarge {
            length: bytes.len(),
        });
    }
    let text = from_utf8(bytes).map_err(|source| RetryAfterError::NotUtf8 { source })?;
    let header = text.trim();
    if let Some(seconds) = parse_seconds(header) {
        return Ok(RetryAfter::Seconds(seconds));
    }
    let date =
        httpdate::parse_http_date(header).map_err(|source| RetryAfterError::NotDate { source })?;
    Ok(RetryAfter::At {
        unix_millis: unix_millis(date),
    })
}

// Python's int() takes a signed run of digits; a magnitude past i64 saturates, and no decision reads past 60
fn parse_seconds(header: &str) -> Option<i64> {
    let negative = header.starts_with('-');
    let digits = header.strip_prefix(['-', '+']).unwrap_or(header);
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let mut magnitude: u64 = 0;
    for digit in digits.bytes() {
        magnitude = magnitude
            .saturating_mul(10)
            .saturating_add(u64::from(digit.saturating_sub(b'0')));
    }
    let limit = i64::MAX.unsigned_abs();
    if negative {
        if magnitude > limit {
            return Some(i64::MIN);
        }
        let value = i64::try_from(magnitude).ok()?;
        return Some(value.saturating_neg());
    }
    let value = i64::try_from(magnitude.min(limit)).ok()?;
    Some(value)
}

pub(crate) fn unix_millis(time: SystemTime) -> i64 {
    match time.duration_since(UNIX_EPOCH) {
        Ok(duration) => i64::try_from(duration.as_millis()).unwrap_or(i64::MAX),
        Err(error) => {
            i64::try_from(error.duration().as_millis()).map_or(i64::MIN, i64::saturating_neg)
        }
    }
}
