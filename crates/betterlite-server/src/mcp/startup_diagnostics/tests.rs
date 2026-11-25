#![cfg(test)]

use std::io;
use std::sync::{Arc, Mutex, PoisonError};

use tracing::Level;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt::{MakeWriter, fmt};

use super::check_separator;

#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl Captured {
    fn contents(&self) -> String {
        let bytes = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        str::from_utf8(&bytes).unwrap().to_owned()
    }
}

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

impl MakeWriter<'_> for Captured {
    type Writer = Self;

    fn make_writer(&self) -> Self {
        self.clone()
    }
}

fn capture_logs() -> (Captured, DefaultGuard) {
    let buffer = Captured::default();
    let subscriber = fmt()
        .with_ansi(false)
        .without_time()
        .with_target(false)
        .with_max_level(Level::WARN)
        .with_writer(buffer.clone())
        .finish();
    (buffer, set_default(subscriber))
}

#[test]
fn valid_separator_silent() {
    let (logs, _guard) = capture_logs();
    check_separator("-");
    check_separator(".");
    check_separator("");
    assert!(logs.contents().is_empty());
}

#[test]
fn invalid_separator_warns() {
    let (logs, _guard) = capture_logs();
    check_separator("/");
    let contents = logs.contents();
    assert!(contents.contains("MCP_TOOL_PREFIX_SEPARATOR"));
    assert!(contents.contains("'/'"));
    assert!(contents.contains(
        "https://modelcontextprotocol.io/specification/2025-11-25/server/tools#tool-names"
    ));
    assert_eq!(contents.matches("WARN").count(), 1);
}

#[test]
fn warning_does_not_fail() {
    let (logs, _guard) = capture_logs();
    check_separator(" ");
    check_separator("*");
    let contents = logs.contents();
    assert_eq!(contents.matches("WARN").count(), 2);
    assert!(!contents.contains("ERROR"));
}

#[test]
fn probe_spans_both_sides() {
    let (logs, _guard) = capture_logs();
    check_separator(&"a".repeat(116));
    assert!(logs.contents().is_empty());
    check_separator(&"a".repeat(117));
    let contents = logs.contents();
    assert!(contents.contains("MCP_TOOL_PREFIX_SEPARATOR"));
    assert_eq!(contents.matches("WARN").count(), 1);
}
