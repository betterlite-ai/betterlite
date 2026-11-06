use betterlite_config::sections::redis_failure_policy as config;
use betterlite_router::redis_failure_policy::log::LogDecision;
use betterlite_router::redis_failure_policy::{BreakerEvent, Settings};

mod tests;

#[must_use]
pub const fn policy_settings(section: &config::Settings) -> Settings {
    Settings {
        enabled: section.enabled,
        failure_threshold: section.failure_threshold,
        recovery_timeout: section.recovery_timeout,
        timeout_min_duration: section.timeout_min_duration,
        timeout_log_interval: section.timeout_log_interval,
    }
}

pub fn log_breaker_event(event: &BreakerEvent) {
    match event {
        BreakerEvent::Opened {
            failure_count,
            hard_failure_count,
            recovery_timeout,
        } => tracing::warn!(
            failure_count,
            hard_failure_count,
            recovery_seconds = recovery_timeout.as_secs(),
            "Redis circuit breaker opened; Redis calls fast-fail until it recovers"
        ),
        BreakerEvent::Recovered => {
            tracing::info!("Redis circuit breaker closed; Redis recovered");
        }
    }
}

pub fn log_redis_failure(decision: LogDecision, message: &str, error: &str) {
    match decision {
        LogDecision::Debug => tracing::debug!(%error, "{message}"),
        LogDecision::Level { suppressed: 0 } => tracing::warn!(%error, "{message}"),
        LogDecision::Level { suppressed } => tracing::warn!(
            %error,
            "{message} ({suppressed} more Redis timeouts since the previous Redis timeout line were logged at DEBUG)"
        ),
    }
}
