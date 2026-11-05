#![cfg(test)]

use std::time::{Duration, Instant};

use super::shared::{CREATED, IN_PROGRESS, UPLOADED, done, sanitizer, vendor};
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
