#![cfg(test)]

use std::io;
use std::io::Write;
use std::sync::{Arc, Mutex};

use betterlite_prompts::arguments::json::{Json, Object};
use tracing::subscriber::with_default;
use tracing_subscriber::fmt;

use super::{TypeSafeError, UNREACHABLE_STATUS_CODE, handle_failure};
use crate::generic_guardrail_contracts::UnreachableFallback;
use crate::typesafe::fixtures::{guardrail, settings};

#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl Write for Captured {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .map_err(|_poisoned| io::Error::other("poisoned"))?
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn capture<T, F>(level: tracing::Level, run: F) -> (T, String)
where
    F: FnOnce() -> T,
{
    let captured = Captured::default();
    let writer = captured.clone();
    let subscriber = fmt()
        .with_max_level(level)
        .with_ansi(false)
        .with_writer(move || writer.clone())
        .finish();
    let result = with_default(subscriber, run);
    let output = String::from_utf8(captured.0.lock().expect("poisoned").clone())
        .expect("tracing output is utf8");
    (result, output)
}

fn detail() -> Object {
    let mut detail = Object::new();
    detail.insert(
        "status_code".to_owned(),
        Json::Number(serde_json::Number::from(503_u16)),
    );
    detail
}

#[test]
fn fail_open_logs_a_warning_with_the_detail_and_allows_the_request() {
    let guardrail = guardrail(settings());
    let (result, logs) = capture(tracing::Level::WARN, || {
        handle_failure(&guardrail, "boom", &detail())
    });
    assert_eq!(result, Ok(()));
    assert!(logs.contains("WARN"));
    assert!(logs.contains("TypeSafe: boom; fail_open configured, forwarding request uncompacted"));
    assert!(logs.contains("status_code"));
}

#[test]
fn fail_closed_logs_an_error_with_the_detail_and_returns_the_unavailable_error() {
    let mut settings = settings();
    settings.unreachable_fallback = UnreachableFallback::FailClosed;
    let guardrail = guardrail(settings);
    let (result, logs) = capture(tracing::Level::ERROR, || {
        handle_failure(&guardrail, "boom", &detail())
    });
    assert_eq!(
        result,
        Err(TypeSafeError::Unavailable {
            message: "boom".to_owned()
        })
    );
    assert!(logs.contains("ERROR"));
    assert!(logs.contains("TypeSafe: boom"));
    assert!(logs.contains("status_code"));
}

#[test]
fn fail_closed_raises_no_warning_line() {
    let mut settings = settings();
    settings.unreachable_fallback = UnreachableFallback::FailClosed;
    let guardrail = guardrail(settings);
    let (_, logs) = capture(tracing::Level::WARN, || {
        handle_failure(&guardrail, "boom", &detail())
    });
    assert!(logs.contains("ERROR"));
    assert!(!logs.contains("fail_open configured"));
}

#[test]
fn the_unreachable_error_status_matches_litellm() {
    assert_eq!(UNREACHABLE_STATUS_CODE, 502);
}
