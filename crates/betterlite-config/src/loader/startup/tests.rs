#![cfg(test)]

use std::io;
use std::path::Path;
use std::sync::{Arc, Mutex};

use tracing::subscriber::with_default;

use super::log_config_error;
use crate::loader::ConfigError;

#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl io::Write for Captured {
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

#[test]
fn a_load_thread_failure_is_logged_with_its_category() {
    let captured = Captured::default();
    let writer = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_writer(move || writer.clone())
        .finish();
    with_default(subscriber, || {
        log_config_error(
            &ConfigError::LoadThread {
                source: io::Error::other("thread limit reached"),
            },
            Path::new("config.yaml"),
        );
    });
    let output = String::from_utf8(
        captured
            .0
            .lock()
            .expect("capture mutex should not be poisoned")
            .clone(),
    )
    .expect("captured log output should be UTF-8");
    assert!(output.contains("failed to load configuration"));
    assert!(output.contains("category=\"load_thread\""));
    assert!(output.contains("config.yaml"));
}
