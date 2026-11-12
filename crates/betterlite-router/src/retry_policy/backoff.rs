use rand::{Rng, RngExt as _};

use crate::retry_policy::eligibility::Availability;

pub const DEFAULT_INITIAL_RETRY_DELAY: f64 = 0.5;
pub const DEFAULT_MAX_RETRY_DELAY: f64 = 8.0;
pub const DEFAULT_JITTER: f64 = 0.75;
const MAX_HEADER_RETRY_AFTER_SECONDS: i64 = 60;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BackoffSettings {
    pub initial_retry_delay: f64,
    pub max_retry_delay: f64,
    pub jitter: f64,
    pub min_timeout: f64,
}

impl Default for BackoffSettings {
    fn default() -> Self {
        Self {
            initial_retry_delay: DEFAULT_INITIAL_RETRY_DELAY,
            max_retry_delay: DEFAULT_MAX_RETRY_DELAY,
            jitter: DEFAULT_JITTER,
            min_timeout: 0.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryTiming {
    pub remaining_retries: i64,
    pub max_retries: i64,
}

// a single deployment always backs off; a healthy alternative retries at once, otherwise back off or obey Retry-After
#[must_use]
pub fn sleep_before_retry<R>(
    settings: &BackoffSettings,
    timing: RetryTiming,
    availability: &Availability,
    retry_after: Option<i64>,
    random: &mut R,
) -> f64
where
    R: Rng + ?Sized,
{
    if availability.all_deployments != 1 && availability.healthy_deployments > 0 {
        return 0.0;
    }
    // float arithmetic operators are denied, so each product is one fused multiply-add
    let jitter = settings.jitter.mul_add(random.random::<f64>(), 0.0);
    if let Some(seconds) =
        retry_after.filter(|seconds| (1..=MAX_HEADER_RETRY_AFTER_SECONDS).contains(seconds))
        && let Ok(header_seconds) = i32::try_from(seconds)
    {
        return f64::from(header_seconds).mul_add(1.0_f64, jitter);
    }
    let exponent = timing.max_retries.saturating_sub(timing.remaining_retries);
    let delay = if exponent > i64::from(i32::MAX) {
        f64::INFINITY
    } else {
        settings.initial_retry_delay.mul_add(
            2.0_f64.powi(i32::try_from(exponent).unwrap_or(i32::MAX)),
            0.0,
        )
    };
    delay
        .max(settings.min_timeout)
        .min(settings.max_retry_delay)
        .mul_add(1.0_f64, jitter)
}
