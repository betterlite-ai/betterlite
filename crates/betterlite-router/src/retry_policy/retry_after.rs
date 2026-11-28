use std::str::{Utf8Error, from_utf8};

use betterlite_core::email_date::{mktime_tz, parsedate_tz, python_int};
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
    #[error("the retry-after value is neither whole seconds nor a date")]
    NotDate,
}

pub fn parse_retry_after(bytes: &[u8]) -> Result<RetryAfter, RetryAfterError> {
    if bytes.len() > MAX_RETRY_AFTER_BYTES {
        return Err(RetryAfterError::TooLarge {
            length: bytes.len(),
        });
    }
    let text = from_utf8(bytes).map_err(|source| RetryAfterError::NotUtf8 { source })?;
    let header = text.trim();
    if let Some(seconds) = python_int(header) {
        return Ok(RetryAfter::Seconds(seconds));
    }
    let parsed = parsedate_tz(header).ok_or(RetryAfterError::NotDate)?;
    let unix_seconds = mktime_tz(&parsed).ok_or(RetryAfterError::NotDate)?;
    Ok(RetryAfter::At {
        unix_millis: unix_seconds.saturating_mul(1000),
    })
}
