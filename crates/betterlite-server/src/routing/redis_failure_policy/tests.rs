#![cfg(test)]

use std::io;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use betterlite_config::sections::redis_failure_policy as config;
use betterlite_router::redis_failure_policy::log::LogDecision;
use betterlite_router::redis_failure_policy::{BreakerEvent, Settings};
use tracing::subscriber::with_default;
use tracing_subscriber::fmt;

use super::{log_breaker_event, log_redis_failure, policy_settings};

struct Captured(Arc<Mutex<Vec<u8>>>);

impl io::Write for Captured {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let Ok(mut output) = self.0.lock() else {
            return Err(io::Error::other("poisoned capture"));
        };
        output.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn capture<F>(log: F) -> String
where
    F: FnOnce(),
{
    let output = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&output);
    let subscriber = fmt()
        .without_time()
        .with_ansi(false)
        .with_writer(move || Captured(Arc::clone(&sink)))
        .finish();
    with_default(subscriber, log);
    let bytes = output.lock().expect("the capture is not poisoned").clone();
    String::from_utf8(bytes).expect("the capture is UTF-8")
}

#[test]
fn policy_settings_carry_every_policy_field() {
    let section = config::Settings {
        enabled: false,
        failure_threshold: 9,
        recovery_timeout: Duration::from_secs(30),
        timeout_min_duration: Duration::from_millis(2_500),
        timeout_log_interval: Duration::from_millis(250),
    };
    let expected = Settings {
        enabled: false,
        failure_threshold: 9,
        recovery_timeout: Duration::from_secs(30),
        timeout_min_duration: Duration::from_millis(2_500),
        timeout_log_interval: Duration::from_millis(250),
    };
    assert_eq!(policy_settings(&section), expected);
}

#[test]
fn an_opened_breaker_logs_a_warning_with_the_failure_counts() {
    let event = BreakerEvent::Opened {
        failure_count: 3,
        hard_failure_count: 3,
        recovery_timeout: Duration::from_secs(30),
    };
    let text = capture(|| log_breaker_event(&event));
    assert!(text.contains("WARN"));
    assert!(text.contains("failure_count=3"));
    assert!(text.contains("hard_failure_count=3"));
    assert!(text.contains("recovery_seconds=30"));
}

#[test]
fn a_recovered_breaker_logs_an_info_line() {
    let text = capture(|| log_breaker_event(&BreakerEvent::Recovered));
    assert!(text.contains("INFO"));
    assert!(text.contains("Redis recovered"));
}

#[test]
fn a_throttled_timeout_line_logs_at_debug() {
    let text = capture(|| log_redis_failure(LogDecision::Debug, "Redis get failed", "timeout"));
    assert!(text.contains("DEBUG"));
    assert!(text.contains("Redis get failed"));
    assert!(text.contains("error=timeout"));
}

#[test]
fn an_admitted_line_logs_at_the_warning_level() {
    let text = capture(|| {
        log_redis_failure(
            LogDecision::Level { suppressed: 0 },
            "Redis get failed",
            "connection refused",
        );
    });
    assert!(text.contains("WARN"));
    assert!(text.contains("Redis get failed"));
    assert!(!text.contains("more Redis timeouts"));
}

#[test]
fn a_suppressed_timeout_count_is_named_in_the_warning() {
    let text = capture(|| {
        log_redis_failure(
            LogDecision::Level { suppressed: 2 },
            "Redis get failed",
            "timeout",
        );
    });
    assert!(text.contains("WARN"));
    assert!(text.contains(
        "(2 more Redis timeouts since the previous Redis timeout line were logged at DEBUG)"
    ));
}
