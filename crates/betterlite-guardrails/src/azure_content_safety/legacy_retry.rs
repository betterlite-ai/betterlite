use std::time::Duration;

use betterlite_core::retry_after::retry_after_from_header;
use jiff::Timestamp;
use reqwest::StatusCode;
use reqwest::header::{HeaderMap, RETRY_AFTER};

// ContentSafetyClient keeps azure-core's default AsyncRetryPolicy: three status retries, 0.8s backoff, 120s cap.
pub const STATUS_RETRIES: u32 = 3;
pub const BACKOFF_FACTOR: Duration = Duration::from_millis(800);
const BACKOFF_MAX: Duration = Duration::from_secs(120);
const RETRY_AFTER_MS: [&str; 2] = ["retry-after-ms", "x-ms-retry-after-ms"];

fn header<'headers>(headers: &'headers HeaderMap, name: &str) -> Option<&'headers str> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .filter(|value| !value.is_empty())
}

// POST is off azure-core's method allowlist, so only 500, 503, 504 or a Retry-After header earn a retry.
#[must_use]
pub fn is_retry(status: StatusCode, headers: &HeaderMap) -> bool {
    status.as_u16() >= 400
        && (header(headers, RETRY_AFTER.as_str()).is_some()
            || matches!(
                status,
                StatusCode::INTERNAL_SERVER_ERROR
                    | StatusCode::SERVICE_UNAVAILABLE
                    | StatusCode::GATEWAY_TIMEOUT
            ))
}

// A Retry-After of zero falls back to the backoff, and the backoff skips the first retry.
#[must_use]
pub fn retry_delay(
    headers: &HeaderMap,
    retries: u32,
    factor: Duration,
    now: Timestamp,
) -> Duration {
    if let Some(delay) = retry_after(headers, now).filter(|delay| !delay.is_zero()) {
        return delay;
    }
    if retries <= 1 {
        return Duration::ZERO;
    }
    factor
        .saturating_mul(2_u32.saturating_pow(retries.saturating_sub(1)))
        .min(BACKOFF_MAX)
}

// Retry-After counts seconds and the two Azure headers milliseconds; a past, negative or unreadable value is zero.
fn retry_after(headers: &HeaderMap, now: Timestamp) -> Option<Duration> {
    if let Some(value) = header(headers, RETRY_AFTER.as_str()) {
        let seconds = u64::try_from(retry_after_from_header(Some(value), now)).unwrap_or(0);
        return Some(Duration::from_secs(seconds));
    }
    let value = RETRY_AFTER_MS
        .iter()
        .find_map(|name| header(headers, name))?;
    let millis = u64::try_from(retry_after_from_header(Some(value), now)).unwrap_or(0);
    Some(Duration::from_millis(millis))
}
