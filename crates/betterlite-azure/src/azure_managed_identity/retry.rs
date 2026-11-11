use std::time::Duration;

use super::error::ManagedIdentityError;
use super::imds::{ImdsRequest, ImdsResponse};
use super::runtime::ImdsIo;

#[derive(Debug, Clone, Copy)]
pub struct RetryBudget {
    pub total: u32,
    pub connect: u32,
}

const BACKOFF_FACTOR_MILLIS: u64 = 800;
const BACKOFF_FACTOR_410_MILLIS: u64 = 2500;
const BACKOFF_MAX_MILLIS: u64 = 60_000;

fn header<'response>(response: &'response ImdsResponse, name: &str) -> Option<&'response str> {
    response
        .headers
        .iter()
        .find(|(header, _)| header.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}

fn retry_after(response: &ImdsResponse) -> Option<Duration> {
    let seconds = header(response, "retry-after").and_then(|value| value.trim().parse().ok());
    if let Some(whole_seconds) = seconds {
        return Some(Duration::from_secs(whole_seconds));
    }
    ["retry-after-ms", "x-ms-retry-after-ms"]
        .iter()
        .find_map(|name| header(response, name)?.trim().parse().ok())
        .map(Duration::from_millis)
}

fn backoff(factor_millis: u64, retries_used: u32) -> Duration {
    if retries_used <= 1 {
        return Duration::ZERO;
    }
    let exponent = retries_used.saturating_sub(1).min(31);
    let millis = factor_millis
        .saturating_mul(1_u64 << exponent)
        .min(BACKOFF_MAX_MILLIS);
    Duration::from_millis(millis)
}

const fn factor(status: u16) -> u64 {
    if status == 410 {
        BACKOFF_FACTOR_410_MILLIS
    } else {
        BACKOFF_FACTOR_MILLIS
    }
}

fn sleep(io: &ImdsIo<'_>, delay: Duration) {
    if !delay.is_zero() {
        io.scheduler.sleep(delay);
    }
}

pub fn send(
    io: &ImdsIo<'_>,
    request: &ImdsRequest,
    budget: RetryBudget,
) -> Result<ImdsResponse, ManagedIdentityError> {
    let mut retries = 0_u32;
    let mut connections = 0_u32;
    loop {
        let response = match io.transport.get(request) {
            Ok(response) => response,
            Err(_source) => {
                if connections >= budget.connect {
                    return Err(ManagedIdentityError::ImdsUnavailable {
                        description: String::new(),
                    });
                }
                connections = connections.saturating_add(1);
                sleep(io, backoff(BACKOFF_FACTOR_MILLIS, connections));
                continue;
            }
        };
        let retry_after = retry_after(&response);
        let retryable = response.status >= 400
            && (retry_after.is_some() || matches!(response.status, 404 | 410 | 429 | 500..=599));
        if !retryable || retries >= budget.total {
            return Ok(response);
        }
        retries = retries.saturating_add(1);
        let delay = retry_after
            .filter(|delay| !delay.is_zero())
            .unwrap_or_else(|| backoff(factor(response.status), retries));
        sleep(io, delay);
    }
}
