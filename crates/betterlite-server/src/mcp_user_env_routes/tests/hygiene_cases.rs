use axum::http::StatusCode;

use super::support::{Harness, MEMBER, call, capture, context, get, post, server_uri};

#[tokio::test]
async fn no_response_body_or_log_line_carries_a_stored_value() {
    let (captured, guard) = capture();
    let harness = Harness::new();
    let app = harness.router();
    let stored = call(
        app.clone(),
        post(
            &server_uri("srv-1"),
            Some(&context(MEMBER)),
            r#"{"values":{"TOKEN":"sk-live-secret"}}"#,
        ),
    )
    .await;
    assert_eq!(stored.status(), StatusCode::OK);
    let read = call(
        app.clone(),
        get(&server_uri("srv-1"), Some(&context(MEMBER))),
    )
    .await;
    assert_eq!(read.status(), StatusCode::OK);
    harness.store().fail_reads();
    let failed = call(
        app.clone(),
        get(&server_uri("srv-1"), Some(&context(MEMBER))),
    )
    .await;
    assert_eq!(failed.status(), StatusCode::INTERNAL_SERVER_ERROR);
    drop(guard);
    assert!(!stored.text().contains("sk-live-secret"));
    assert!(!read.text().contains("sk-live-secret"));
    assert!(!failed.text().contains("sk-live-secret"));
    let logs = captured.text();
    assert!(logs.contains("the MCP user env vars request failed"));
    assert!(logs.contains("the per-user env vars could not be read or written"));
    assert!(!logs.contains("sk-live-secret"));
}
