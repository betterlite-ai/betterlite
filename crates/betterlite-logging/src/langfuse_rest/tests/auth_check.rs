use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio_util::task::TaskTracker;

use super::support::{
    Answer, FakeClock, FakeLangfuse, NO_PROJECTS, PROJECTS, capture_logs, client_for,
};

const TTL: Duration = Duration::from_secs(60);
const UNAUTHORIZED: &str = r#"{"message":"Unauthorized"}"#;

#[tokio::test]
async fn auth_check_passes_when_a_project_is_returned() {
    let server = FakeLangfuse::start(vec![Answer::Respond {
        status: 200,
        body: PROJECTS.to_owned(),
    }])
    .await;
    let tasks = TaskTracker::new();
    let clock = FakeClock(Arc::new(Mutex::new(Instant::now())));
    let client = client_for(&server, TTL, &clock, &tasks);
    assert!(client.auth_check().await.is_none());
    assert_eq!(server.request_count(), 1);
    assert_eq!(
        server.requests()[0].lines().next().expect("a request line"),
        "GET /api/public/projects HTTP/1.1"
    );
}

#[tokio::test]
async fn auth_check_reports_an_empty_project_list() {
    let (captured, _guard) = capture_logs();
    let server = FakeLangfuse::start(vec![Answer::Respond {
        status: 200,
        body: NO_PROJECTS.to_owned(),
    }])
    .await;
    let tasks = TaskTracker::new();
    let clock = FakeClock(Arc::new(Mutex::new(Instant::now())));
    let client = client_for(&server, TTL, &clock, &tasks);
    let failure = client.auth_check().await.expect("the check fails");
    assert_eq!(failure.reason, "no project found for the keys provided");
    let logs = captured.text();
    assert!(
        logs.contains("Langfuse auth check failed: no project found for the keys provided"),
        "{logs}"
    );
}

#[tokio::test]
async fn auth_check_reports_bad_credentials_with_status_and_body() {
    let (captured, _guard) = capture_logs();
    let server = FakeLangfuse::start(vec![Answer::Respond {
        status: 401,
        body: UNAUTHORIZED.to_owned(),
    }])
    .await;
    let tasks = TaskTracker::new();
    let clock = FakeClock(Arc::new(Mutex::new(Instant::now())));
    let client = client_for(&server, TTL, &clock, &tasks);
    let failure = client.auth_check().await.expect("the check fails");
    assert!(
        failure.reason.contains("status_code: 401"),
        "{}",
        failure.reason
    );
    assert!(
        failure.reason.contains("Unauthorized"),
        "{}",
        failure.reason
    );
    let logs = captured.text();
    assert!(logs.contains("Langfuse auth check failed:"), "{logs}");
    assert!(logs.contains("status_code: 401"), "{logs}");
}

#[tokio::test]
async fn auth_check_reports_a_server_error_as_itself() {
    let server = FakeLangfuse::start(vec![Answer::Respond {
        status: 500,
        body: r#"{"message":"boom"}"#.to_owned(),
    }])
    .await;
    let tasks = TaskTracker::new();
    let clock = FakeClock(Arc::new(Mutex::new(Instant::now())));
    let client = client_for(&server, TTL, &clock, &tasks);
    let failure = client.auth_check().await.expect("the check fails");
    assert!(
        failure.reason.contains("status_code: 500"),
        "{}",
        failure.reason
    );
    assert!(
        !failure.reason.contains("no project found"),
        "{}",
        failure.reason
    );
}

#[tokio::test]
async fn auth_check_reports_a_transport_failure() {
    let (captured, _guard) = capture_logs();
    let server = FakeLangfuse::start(vec![Answer::Close]).await;
    let tasks = TaskTracker::new();
    let clock = FakeClock(Arc::new(Mutex::new(Instant::now())));
    let client = client_for(&server, TTL, &clock, &tasks);
    let failure = client.auth_check().await.expect("the check fails");
    assert!(!failure.reason.is_empty());
    let logs = captured.text();
    assert!(logs.contains("Langfuse auth check failed:"), "{logs}");
}

#[tokio::test]
async fn auth_check_reports_a_response_body_that_is_not_json() {
    let server = FakeLangfuse::start(vec![Answer::Respond {
        status: 200,
        body: "not json".to_owned(),
    }])
    .await;
    let tasks = TaskTracker::new();
    let clock = FakeClock(Arc::new(Mutex::new(Instant::now())));
    let client = client_for(&server, TTL, &clock, &tasks);
    let failure = client.auth_check().await.expect("the check fails");
    assert!(
        failure.reason.contains("status_code: 200"),
        "{}",
        failure.reason
    );
    assert!(failure.reason.contains("not json"), "{}", failure.reason);
}

#[tokio::test]
async fn auth_check_does_not_expose_response_headers() {
    let server = FakeLangfuse::start(vec![Answer::RespondWith {
        status: 401,
        body: UNAUTHORIZED.to_owned(),
        headers: vec![("x-langfuse-secret", "leaky-value".to_owned())],
    }])
    .await;
    let tasks = TaskTracker::new();
    let clock = FakeClock(Arc::new(Mutex::new(Instant::now())));
    let client = client_for(&server, TTL, &clock, &tasks);
    let failure = client.auth_check().await.expect("the check fails");
    assert!(
        !failure.reason.contains("leaky-value"),
        "{}",
        failure.reason
    );
    assert!(
        !failure.reason.contains("x-langfuse-secret"),
        "{}",
        failure.reason
    );
}

#[tokio::test]
async fn auth_check_does_not_retry_a_server_error() {
    let server = FakeLangfuse::start(vec![Answer::Respond {
        status: 500,
        body: r#"{"message":"boom"}"#.to_owned(),
    }])
    .await;
    let tasks = TaskTracker::new();
    let clock = FakeClock(Arc::new(Mutex::new(Instant::now())));
    let client = client_for(&server, TTL, &clock, &tasks);
    assert!(client.auth_check().await.is_some());
    assert_eq!(server.request_count(), 1);
}
