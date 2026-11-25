use std::time::Duration;

use super::BreakerEvent;

// LiteLLM's call sites pass logging.ERROR or logging.WARNING, and nothing else
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FailureLogLevel {
    Warning,
    Error,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LogDecision {
    Debug,
    Level { suppressed: u32 },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TimeoutLogThrottle {
    pub interval: Duration,
    pub last_logged_at: Option<Duration>,
    pub suppressed: u32,
}

impl TimeoutLogThrottle {
    #[must_use]
    pub fn admit(&mut self, now: Duration) -> Option<u32> {
        if let Some(last) = self.last_logged_at
            && now.saturating_sub(last) < self.interval
        {
            self.suppressed = self.suppressed.saturating_add(1);
            return None;
        }
        let suppressed = self.suppressed;
        self.suppressed = 0;
        self.last_logged_at = Some(now);
        Some(suppressed)
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
