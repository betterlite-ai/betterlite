use std::collections::BTreeMap;
use std::fmt;

mod tests;

pub const FAIL_CLOSED_SETTING: &str = "fail_closed_rate_limit_enforcement";

pub const UNVERIFIABLE_MESSAGE: &str = "Rate limit enforcement unavailable: request counters could not be verified against Redis, and fail_closed_rate_limit_enforcement is enabled, so the request was rejected to avoid exceeding the configured rate limit. Retry shortly.";

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
#[error("{}", UNVERIFIABLE_MESSAGE)]
pub struct RateLimitUnverifiableError {
    pub status_code: u16,
    pub detail: BTreeMap<String, String>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct FailClosedPolicy {
    pub enabled: bool,
}

impl FailClosedPolicy {
    pub fn reject_if_unverifiable<E>(
        &self,
        failed_operation: &str,
        error: &E,
    ) -> Result<(), RateLimitUnverifiableError>
    where
        E: fmt::Display + ?Sized,
    {
        if !self.enabled {
            return Ok(());
        }
        tracing::warn!(
            %error,
            "fail_closed_rate_limit_enforcement: rejecting request, {} could not verify the counters against Redis",
            failed_operation
        );
        Err(RateLimitUnverifiableError {
            status_code: 503,
            detail: BTreeMap::from([("error".to_owned(), UNVERIFIABLE_MESSAGE.to_owned())]),
        })
    }
}
