use std::time::Duration;

use chrono::DateTime;

use crate::azure_arc_identity::{MetadataRequest, MetadataResponse};

use super::error::ManagedIdentityError;
use super::runtime::ImdsIo;

#[derive(Debug, Clone, Copy)]
pub struct RetryBudget {
    pub total: u32,
    pub connect: u32,
    pub status: u32,
}

const BACKOFF_FACTOR_MILLIS: u64 = 800;
const BACKOFF_FACTOR_410_MILLIS: u64 = 2500;
const BACKOFF_MAX_MILLIS: u64 = 60_000;

#[derive(Clone, Copy)]
enum Failure {
    Connect,
    Status,
}

// azure-core counts every retry against the total, its own kind and one shared history
struct Counters {
    total: i64,
    connect: i64,
    status: i64,
    history: u32,
}

impl Counters {
    // a counter that reaches zero still allows the retry, the next decrement below zero ends them
    const fn increment(&mut self, failure: Failure) -> bool {
        self.total = self.total.saturating_sub(1);
        match failure {
            Failure::Connect => self.connect = self.connect.saturating_sub(1),
            Failure::Status => self.status = self.status.saturating_sub(1),
        }
        self.history = self.history.saturating_add(1);
        self.total >= 0 && self.connect >= 0 && self.status >= 0
    }
}

fn header<'response>(response: &'response MetadataResponse, name: &str) -> Option<&'response str> {
    response
        .headers
        .iter()
        .find(|(header, _)| header.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
        .filter(|value| !value.is_empty())
}

// a Retry-After header is either a number of seconds or an HTTP date; anything else waits no longer than the backoff
fn parse_delay(value: &str, now: i64) -> Duration {
    let trimmed = value.trim();
    if let Ok(seconds) = trimmed.parse::<f64>() {
        return Duration::try_from_secs_f64(seconds.max(0.0)).unwrap_or(Duration::ZERO);
    }
    DateTime::parse_from_rfc2822(trimmed).map_or(Duration::ZERO, |date| {
        u64::try_from(date.timestamp().saturating_sub(now))
            .map_or(Duration::ZERO, Duration::from_secs)
    })
}

fn retry_after(response: &MetadataResponse, now: i64) -> Option<Duration> {
    if let Some(value) = header(response, "retry-after") {
        return Some(parse_delay(value, now));
    }
    ["retry-after-ms", "x-ms-retry-after-ms"]
        .iter()
        .find_map(|name| header(response, name))
        .map(|value| parse_delay(value, now).div_f64(1000.0))
}

// a Retry-After header makes any error status retryable, even once the total reached zero
fn is_retry(response: &MetadataResponse, total: i64) -> bool {
    if response.status < 400 {
        return false;
    }
    header(response, "retry-after").is_some()
        || (total != 0 && matches!(response.status, 404 | 408 | 410 | 429 | 500..=998))
}

fn backoff(factor_millis: u64, history: u32) -> Duration {
    if history <= 1 {
        return Duration::ZERO;
    }
    let exponent = history.saturating_sub(1).min(31);
    let millis = factor_millis
        .saturating_mul(1_u64 << exponent)
        .min(BACKOFF_MAX_MILLIS);
    Duration::from_millis(millis)
}

fn sleep(io: &ImdsIo<'_>, delay: Duration) {
    if !delay.is_zero() {
        io.scheduler.sleep(delay);
    }
}

// the IMDS policy picks its backoff factor from the last response and keeps it for later connection errors
pub fn send(
    io: &ImdsIo<'_>,
    request: &MetadataRequest,
    budget: RetryBudget,
) -> Result<MetadataResponse, ManagedIdentityError> {
    let mut counters = Counters {
        total: i64::from(budget.total),
        connect: i64::from(budget.connect),
        status: i64::from(budget.status),
        history: 0,
    };
    let mut factor_millis = BACKOFF_FACTOR_MILLIS;
    loop {
        let Ok(response) = io.transport.get(request) else {
            if !counters.increment(Failure::Connect) {
                return Err(ManagedIdentityError::ImdsUnavailable);
            }
            sleep(io, backoff(factor_millis, counters.history));
            continue;
        };
        factor_millis = if response.status == 410 {
            BACKOFF_FACTOR_410_MILLIS
        } else {
            BACKOFF_FACTOR_MILLIS
        };
        if !is_retry(&response, counters.total) || !counters.increment(Failure::Status) {
            return Ok(response);
        }
        let delay = retry_after(&response, io.clock.now_unix_seconds())
            .filter(|delay| !delay.is_zero())
            .unwrap_or_else(|| backoff(factor_millis, counters.history));
        sleep(io, delay);
    }
}
