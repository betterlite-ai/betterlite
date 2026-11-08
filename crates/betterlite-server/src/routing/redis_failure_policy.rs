use betterlite_config::sections::redis_failure_policy as config;
use betterlite_router::redis_failure_policy::log::LogDecision;
use betterlite_router::redis_failure_policy::{BreakerEvent, Settings};

mod tests;

// LiteLLM's call sites pass logging.ERROR or logging.WARNING, and nothing else
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FailureLogLevel {
    Warning,
    Error,
}

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

pub fn log_redis_failure(
    decision: LogDecision,
    level: FailureLogLevel,
    message: &str,
    error: &str,
) {
    let rendered = match decision {
        LogDecision::Debug => {
            tracing::debug!(%error, "{message}");
            return;
        }
        LogDecision::Level { suppressed: 0 } => message.to_owned(),
        LogDecision::Level { suppressed } => format!(
            "{message} ({suppressed} more Redis timeouts since the previous Redis timeout line were logged at DEBUG)"
        ),
    };
    match level {
        FailureLogLevel::Warning => tracing::warn!(%error, "{rendered}"),
        FailureLogLevel::Error => tracing::error!(%error, "{rendered}"),
    }
}
