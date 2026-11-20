use std::time::{Duration, Instant};

use betterlite_logging::prometheus_helpers::bounded_series_tracker::SeriesLimits;
use betterlite_oracle_helper_api::error::HelperError;

use super::helper_error;

pub(super) fn series_limits(
    max_series: Option<i64>,
    ttl_seconds: Option<f64>,
    cleanup_interval_seconds: Option<f64>,
) -> Result<SeriesLimits, Box<HelperError>> {
    Ok(SeriesLimits {
        max_series,
        ttl: ttl_seconds.map(seconds_duration).transpose()?,
        cleanup_interval: cleanup_interval_seconds.map(cleanup_duration).transpose()?,
    })
}

pub(super) fn seconds_duration(seconds: f64) -> Result<Duration, Box<HelperError>> {
    Duration::try_from_secs_f64(seconds)
        .map_err(|error| helper_error("InvalidInput", &format!("invalid seconds: {error}")))
}

pub(super) fn cleanup_duration(seconds: f64) -> Result<Duration, Box<HelperError>> {
    if seconds <= 0.0 {
        return Ok(Duration::ZERO);
    }
    seconds_duration(seconds)
}

pub(super) fn time_at(base: Instant, seconds: f64) -> Result<Instant, Box<HelperError>> {
    let offset = seconds_duration(seconds)?;
    base.checked_add(offset)
        .ok_or_else(|| helper_error("InvalidInput", "the emission time is out of range"))
}
