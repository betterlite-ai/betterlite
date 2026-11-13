#![cfg(test)]

use std::collections::BTreeMap;
use std::io;
use std::sync::{Arc, Mutex};

use tracing::Level;
use tracing::subscriber::with_default;
use tracing_subscriber::fmt;

use super::{
    FAIL_CLOSED_SETTING, FailClosedPolicy, RateLimitUnverifiableError, UNVERIFIABLE_MESSAGE,
};

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
        .with_max_level(Level::TRACE)
        .with_writer(move || Captured(Arc::clone(&sink)))
        .finish();
    with_default(subscriber, log);
    let bytes = output.lock().expect("the capture is not poisoned").clone();
    String::from_utf8(bytes).expect("the capture is UTF-8")
}

fn disabled() -> FailClosedPolicy {
    FailClosedPolicy { enabled: false }
}

fn enabled() -> FailClosedPolicy {
    FailClosedPolicy { enabled: true }
}

#[test]
fn the_setting_name_matches_litellms_general_settings_key() {
    assert_eq!(FAIL_CLOSED_SETTING, "fail_closed_rate_limit_enforcement");
}

#[test]
fn the_message_is_copied_from_litellm() {
    assert_eq!(
        UNVERIFIABLE_MESSAGE,
        "Rate limit enforcement unavailable: request counters could not be verified against Redis, and fail_closed_rate_limit_enforcement is enabled, so the request was rejected to avoid exceeding the configured rate limit. Retry shortly."
    );
}

#[test]
fn the_error_carries_status_503_and_the_message_detail() {
    let error: RateLimitUnverifiableError = enabled()
        .reject_if_unverifiable("batch_counter_read_script", &"connection refused")
        .expect_err("an enabled policy rejects the request");
    assert_eq!(error.status_code, 503);
    assert_eq!(
        error.detail,
        BTreeMap::from([("error".to_owned(), UNVERIFIABLE_MESSAGE.to_owned())])
    );
    assert_eq!(error.to_string(), UNVERIFIABLE_MESSAGE);
}

#[test]
fn a_disabled_policy_returns_ok_and_logs_nothing() {
    let text = capture(|| {
        assert_eq!(
            disabled().reject_if_unverifiable("batch_rate_limiter_script", &"timeout"),
            Ok(())
        );
    });
    assert!(text.is_empty());
}

#[test]
fn an_enabled_policy_logs_a_warning_and_returns_the_error() {
    let text = capture(|| {
        let error = enabled()
            .reject_if_unverifiable("batch_rate_limiter_script", &"timeout")
            .expect_err("an enabled policy rejects the request");
        assert_eq!(error.status_code, 503);
    });
    assert!(text.contains("WARN"));
    assert!(text.contains(
        "fail_closed_rate_limit_enforcement: rejecting request, batch_rate_limiter_script could not verify the counters against Redis"
    ));
    assert!(text.contains("error=timeout"));
}
