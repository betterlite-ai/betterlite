#![cfg(test)]

use std::collections::BTreeMap;

use tracing::Level;
use tracing::subscriber::with_default;

use super::shared::Capture;
use crate::prompt_security_files::log::log_api_request;

#[test]
fn log_request_records_method_url_payload_and_redacted_headers() {
    let capture = Capture::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(capture.clone())
        .with_max_level(Level::DEBUG)
        .with_ansi(false)
        .finish();
    let headers = BTreeMap::from([
        ("APP-ID".to_owned(), "secret-app-id".to_owned()),
        ("X-LiteLLM-Key-Alias".to_owned(), "alias".to_owned()),
    ]);
    with_default(subscriber, || {
        log_api_request(
            "POST",
            "https://vendor.test/api/sanitizeFile",
            &headers,
            "payload-text",
        );
    });
    let logged = capture.text();
    assert!(logged.contains("Prompt Security request"));
    assert!(logged.contains("POST"));
    assert!(logged.contains("https://vendor.test/api/sanitizeFile"));
    assert!(logged.contains("payload-text"));
    assert!(logged.contains("alias"));
    assert!(logged.contains("REDACTED"));
    assert!(!logged.contains("secret-app-id"));
}

#[test]
fn log_request_is_silent_above_debug_level() {
    let capture = Capture::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(capture.clone())
        .with_max_level(Level::INFO)
        .finish();
    with_default(subscriber, || {
        log_api_request("GET", "https://vendor.test", &BTreeMap::new(), "");
    });
    assert_eq!(capture.text(), "");
}
