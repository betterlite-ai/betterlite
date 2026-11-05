#![cfg(test)]
use super::{failure_row_traceback, log_spend_error, should_suppress_tracebacks};
use betterlite_config::collector::{TracebackPolicy, load_spend_traceback_policy};
use std::io;
use std::sync::{Arc, Mutex, PoisonError};
use tokio::sync::watch;

#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);
impl io::Write for Captured {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn loads_spend_traceback_policy() {
    let (sender, snapshot) = watch::channel(TracebackPolicy::default());
    assert!(!should_suppress_tracebacks(&snapshot, false));
    sender.send_replace(load_spend_traceback_policy(Some(" TRUE ")).unwrap());
    assert!(should_suppress_tracebacks(&snapshot, false));
    assert!(!should_suppress_tracebacks(&snapshot, true));
    assert_eq!(
        failure_row_traceback(&snapshot, Some("trace".to_owned()), false),
        None
    );
    assert_eq!(
        failure_row_traceback(&snapshot, Some("trace".to_owned()), true),
        Some("trace".to_owned())
    );
    sender.send_replace(load_spend_traceback_policy(None).unwrap());
    assert!(!should_suppress_tracebacks(&snapshot, false));
}

#[rstest::rstest]
#[case::yes("yes", true)]
#[case::one("1", true)]
#[case::false_value("false", false)]
#[case::empty("", false)]
fn follows_shared_environment_boolean_policy(#[case] value: &str, #[case] expected: bool) {
    assert_eq!(
        load_spend_traceback_policy(Some(value)).unwrap().suppress,
        expected
    );
    load_spend_traceback_policy(Some("invalid")).unwrap_err();
}

#[test]
fn spend_logging_preserves_supplied_or_active_trace() {
    use tracing::subscriber::with_default;
    let captured = Captured::default();
    let writer = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_max_level(tracing::Level::INFO)
        .with_writer(move || writer.clone())
        .finish();
    let (sender, settings) = watch::channel(TracebackPolicy { suppress: true });
    with_default(subscriber, || {
        log_spend_error(&settings, "suppressed", Some("hidden"), None);
        sender.send_replace(TracebackPolicy { suppress: false });
        log_spend_error(&settings, "explicit", Some("supplied"), Some("unused"));
        log_spend_error(&settings, "implicit", None, Some("active"));
    });
    let logs = String::from_utf8(captured.0.lock().unwrap().clone()).unwrap();
    assert!(!logs.contains("hidden"));
    assert!(!logs.contains("unused"));
    assert!(logs.contains("supplied"));
    assert!(logs.contains("active"));
}
