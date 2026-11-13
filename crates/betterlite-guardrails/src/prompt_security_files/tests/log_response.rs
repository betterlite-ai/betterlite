#![cfg(test)]

use tracing::Level;
use tracing::subscriber::with_default;

use super::shared::Capture;
use crate::prompt_security_files::log::log_api_response;

#[test]
fn log_response_records_url_status_and_payload_under_the_guardrail_name() {
    let capture = Capture::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(capture.clone())
        .with_max_level(Level::DEBUG)
        .with_ansi(false)
        .finish();
    with_default(subscriber, || {
        log_api_response("https://vendor.test/api/sanitizeFile", 200, "body-text");
    });
    let logged = capture.text();
    assert!(logged.contains("Prompt Security response"));
    assert!(logged.contains("prompt_security"));
    assert!(logged.contains("https://vendor.test/api/sanitizeFile"));
    assert!(logged.contains("200"));
    assert!(logged.contains("body-text"));
}

#[test]
fn log_response_is_silent_above_debug_level() {
    let capture = Capture::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(capture.clone())
        .with_max_level(Level::INFO)
        .finish();
    with_default(subscriber, || {
        log_api_response("https://vendor.test", 500, "x");
    });
    assert_eq!(capture.text(), "");
}
