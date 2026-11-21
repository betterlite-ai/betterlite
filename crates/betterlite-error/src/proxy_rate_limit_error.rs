use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use betterlite_redact::structured::Payload;
use chrono::{DateTime, Utc};

use crate::rate_limit::{
    InvalidRateLimitHeader, ProxyRateLimitError, ProxyRateLimitErrorInit, RateLimitDimension,
};

mod tests;

// map_v3_rate_limit_type: the v3 limiter's internal names collapse onto the public dimension, and an unrecognized one leaves the field absent.
#[must_use]
pub fn map_v3_rate_limit_type(v3_value: Option<&str>) -> Option<RateLimitDimension> {
    match v3_value {
        Some("tokens") => Some(RateLimitDimension::Tokens),
        Some("max_parallel_requests") => Some(RateLimitDimension::ConcurrentRequests),
        Some("requests") => Some(RateLimitDimension::Requests),
        Some(_) | None => None,
    }
}

// _handle_rate_limit_error's reset moment: the limiter's clock plus the window it was enforcing.
#[derive(Clone, Copy, Debug)]
pub struct RateLimitResetWindow {
    pub now: SystemTime,
    pub window_size: i64,
}

// Why a v3 rejection can't become a ProxyRateLimitError: a clock or reset time no UTC timestamp can show, or a header HTTP refuses.
#[derive(Debug, thiserror::Error)]
pub enum ProxyRateLimitProjectionError {
    #[error("the rate limiter's clock reads before 1970-01-01T00:00:00Z")]
    ClockBeforeEpoch,
    #[error("the rate limit reset time cannot be shown as a UTC timestamp")]
    ResetOutOfRange,
    #[error(transparent)]
    Header(#[from] InvalidRateLimitHeader),
}

impl RateLimitResetWindow {
    // datetime.fromtimestamp(now + window_size, tz=timezone.utc).strftime("%Y-%m-%d %H:%M:%S UTC"), with the fraction of a second dropped.
    fn reset_at(&self) -> Result<String, ProxyRateLimitProjectionError> {
        let elapsed = self
            .now
            .duration_since(UNIX_EPOCH)
            .map_err(|_error| ProxyRateLimitProjectionError::ClockBeforeEpoch)?;
        let seconds = i64::try_from(elapsed.as_secs())
            .ok()
            .and_then(|now_seconds| now_seconds.checked_add(self.window_size))
            .ok_or(ProxyRateLimitProjectionError::ResetOutOfRange)?;
        DateTime::<Utc>::from_timestamp(seconds, 0)
            .map(|moment| moment.format("%Y-%m-%d %H:%M:%S UTC").to_string())
            .ok_or(ProxyRateLimitProjectionError::ResetOutOfRange)
    }
}

// The OVER_LIMIT status _handle_rate_limit_error projects, with the descriptor value the limiter matched.
#[derive(Clone, Copy, Debug)]
pub struct V3RateLimitExceeded<'text> {
    pub descriptor_key: &'text str,
    pub descriptor_value: &'text str,
    pub rate_limit_type: &'text str,
    pub current_limit: i64,
    pub limit_remaining: i64,
}

impl V3RateLimitExceeded<'_> {
    fn detail(&self, reset_at: &str) -> String {
        let remaining = self.limit_remaining.max(0);
        format!(
            "Rate limit exceeded for {}: {}. Limit type: {}. Current limit: {}, Remaining: {remaining}. Limit resets at: {reset_at}",
            self.descriptor_key, self.descriptor_value, self.rate_limit_type, self.current_limit
        )
    }

    fn headers(&self, window_size: i64, reset_at: &str) -> BTreeMap<String, Payload> {
        let mut headers = BTreeMap::new();
        for (name, value) in [
            ("retry-after", window_size.to_string()),
            ("rate_limit_type", self.rate_limit_type.to_owned()),
            ("reset_at", reset_at.to_owned()),
        ] {
            headers.insert(String::from(name), Payload::String(value));
        }
        headers
    }
}

// _handle_rate_limit_error: the 429 a v3 limiter rejection raises, with its detail text, its three headers and the public dimension.
pub fn v3_rate_limit_error(
    exceeded: &V3RateLimitExceeded<'_>,
    window: RateLimitResetWindow,
    model: String,
    llm_provider: String,
) -> Result<ProxyRateLimitError, ProxyRateLimitProjectionError> {
    let reset_at = window.reset_at()?;
    let init = ProxyRateLimitErrorInit {
        detail: Payload::String(exceeded.detail(&reset_at)),
        headers: Some(exceeded.headers(window.window_size, &reset_at)),
        category: None,
        rate_limit_type: map_v3_rate_limit_type(Some(exceeded.rate_limit_type)),
        model: Some(model),
        llm_provider: Some(llm_provider),
    };
    ProxyRateLimitError::new(init).map_err(ProxyRateLimitProjectionError::Header)
}
