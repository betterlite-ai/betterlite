use std::io;
use std::sync::{Arc, Mutex, PoisonError};

use tracing::subscriber::{DefaultGuard, set_default};

use super::parse;

#[test]
fn logs_and_ignores_unsupported_transcription_settings() {
    let (captured, _guard) = capture_debug();
    let parsed = parse(
        r#"{"type":"session.update","session":{"audio":{"input":{"transcription":{"model":"chirp_2","prompt":"p","temperature":0.5}}}}}"#,
    )
    .unwrap();
    assert_eq!(parsed.model, "chirp_2");
    let logs = captured.take();
    assert!(
        logs.contains("ignoring unsupported transcription settings"),
        "logs: {logs}"
    );
    assert!(logs.contains("prompt"), "logs: {logs}");
    assert!(logs.contains("temperature"), "logs: {logs}");
}

#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl Captured {
    fn take(&self) -> String {
        let bytes = self
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        String::from_utf8(bytes).expect("captured logs are utf-8")
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

fn capture_debug() -> (Captured, DefaultGuard) {
    let captured = Captured::default();
    let writer = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(move || writer.clone())
        .with_max_level(tracing::Level::DEBUG)
        .with_ansi(false)
        .finish();
    (captured, set_default(subscriber))
}
