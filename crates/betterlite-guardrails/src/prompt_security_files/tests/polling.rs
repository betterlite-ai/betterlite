#![cfg(test)]

use std::time::{Duration, Instant};

use tracing::Level;
use tracing::subscriber::set_default;

use super::shared::{CREATED, Capture, DONE_ALLOW, IN_PROGRESS, UPLOADED, done, sanitizer, vendor};
use crate::prompt_security_files::wire::Action;

#[tokio::test]
async fn polling_keeps_going_through_queued_statuses_until_done() {
    let polls = [
        CREATED.to_owned(),
        IN_PROGRESS.to_owned(),
        done("modify", "clean"),
    ];
    let server = vendor(UPLOADED, &polls).await;
    let result = sanitizer(&server.base)
        .sanitize_file_content(b"a", "image.png", None)
        .await
        .expect("done");
    assert_eq!(result.action, Action::Modify);
    assert_eq!(result.content.as_deref(), Some("clean"));
    assert_eq!(result.violations, ["secret", "pii"]);
    assert_eq!(server.requests("GET").len(), 3);
}

#[tokio::test]
async fn polling_waits_the_poll_interval_before_each_attempt() {
    let server = vendor(UPLOADED, &[CREATED.to_owned(), done("allow", "x")]).await;
    let mut guardrail = sanitizer(&server.base);
    guardrail.settings.poll_interval = Duration::from_millis(40);
    let started = Instant::now();
    guardrail
        .sanitize_file_content(b"a", "image.png", None)
        .await
        .expect("done");
    assert!(started.elapsed() >= Duration::from_millis(80));
}

#[tokio::test]
async fn polling_keeps_the_vendor_metadata_on_the_result() {
    let server = vendor(UPLOADED, &[done("modify", "clean")]).await;
    let result = sanitizer(&server.base)
        .sanitize_file_content(b"a", "image.png", None)
        .await
        .expect("done");
    assert_eq!(result.metadata.action, Some(Action::Modify));
    assert_eq!(
        result.metadata.violations,
        Some(vec!["secret".to_owned(), "pii".to_owned()])
    );
}

#[tokio::test]
async fn polling_logs_completion_at_debug_level() {
    let server = vendor(UPLOADED, &[DONE_ALLOW.to_owned()]).await;
    let capture = Capture::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(capture.clone())
        .with_max_level(Level::DEBUG)
        .with_ansi(false)
        .finish();
    let _guard = set_default(subscriber);
    sanitizer(&server.base)
        .sanitize_file_content(b"a", "image.png", None)
        .await
        .expect("done");
    assert!(capture.text().contains("File sanitization completed"));
}

#[tokio::test]
async fn polling_defaults_to_allow_without_metadata() {
    let server = vendor(UPLOADED, &[r#"{"status":"done"}"#.to_owned()]).await;
    let result = sanitizer(&server.base)
        .sanitize_file_content(b"a", "image.png", None)
        .await
        .expect("done");
    assert_eq!(result.action, Action::Allow);
    assert_eq!(result.content, None);
    assert!(result.violations.is_empty());
}
