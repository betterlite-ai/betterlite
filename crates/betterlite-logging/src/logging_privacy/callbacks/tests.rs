#![cfg(test)]

use crate::logging_privacy::callbacks::callback_metadata;
use betterlite_models::decisions::Json;
use rstest::rstest;
use std::io::{self, Write};
use std::sync::{Arc, Mutex, PoisonError};
use tracing::subscriber::with_default;

#[derive(Clone)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl Write for Captured {
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

#[rstest]
#[case::blocked_credential("dd_api_key")]
#[case::blocked_destination("gcs_bucket_name")]
#[case::configured_destination("langsmith_base_url")]
fn logging_privacy_tests_behavior(#[case] field: &str) {
    let buffer = Arc::new(Mutex::new(Vec::new()));
    let captured = Captured(Arc::clone(&buffer));
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_writer(move || captured.clone())
        .finish();
    let payload: Json = serde_json::from_str(&format!(r#"{{"{field}":"private-value"}}"#)).unwrap();
    with_default(subscriber, || {
        let safe = callback_metadata(&payload).unwrap();
        assert!(safe.fields.is_empty());
        assert!(!format!("{safe:?}").contains("private-value"));
    });
    let output = String::from_utf8(buffer.lock().unwrap().clone()).unwrap();
    assert!(output.contains(field));
    assert!(!output.contains("private-value"));
}
