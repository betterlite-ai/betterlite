use std::time::Duration;

pub const MIN_HANGING_POLL_INTERVAL: Duration = Duration::from_secs(1);

#[must_use]
pub fn poll_interval(alerting_threshold_seconds: f64) -> Duration {
    let seconds = alerting_threshold_seconds.mul_add(0.5, 0.0);
    let interval = Duration::try_from_secs_f64(seconds).unwrap_or(Duration::ZERO);
    interval.max(MIN_HANGING_POLL_INTERVAL)
}
