use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use betterlite_models::decisions::Json;
use tokio_util::task::TaskTracker;

use super::support::{
    Answer, FakeClock, FakeLangfuse, NO_PROJECTS, PROJECTS, api_error, client_for,
};
use crate::langfuse_rest::errors::LangfuseRestError;
use crate::langfuse_rest::transport::RequestFailure;

const TTL: Duration = Duration::from_secs(60);

#[tokio::test]
async fn project_id_returns_the_first_project() {
    let server = FakeLangfuse::start(vec![Answer::Respond {
        status: 200,
        body: PROJECTS.to_owned(),
    }])
    .await;
    let tasks = TaskTracker::new();
    let clock = FakeClock(Arc::new(Mutex::new(Instant::now())));
    let client = client_for(&server, TTL, &clock, &tasks);
    let project_id = client.project_id().await.expect("the lookup");
    assert_eq!(project_id.as_deref(), Some("project-one"));
    assert_eq!(server.request_count(), 1);
    assert_eq!(
        server.requests()[0].lines().next().expect("a request line"),
        "GET /api/public/projects HTTP/1.1"
    );
}

#[tokio::test]
async fn project_id_is_none_when_the_project_list_is_empty() {
    let server = FakeLangfuse::start(vec![Answer::Respond {
        status: 200,
        body: NO_PROJECTS.to_owned(),
    }])
    .await;
    let tasks = TaskTracker::new();
    let clock = FakeClock(Arc::new(Mutex::new(Instant::now())));
    let client = client_for(&server, TTL, &clock, &tasks);
    assert_eq!(client.project_id().await.expect("the lookup"), None);
}

#[tokio::test]
async fn project_id_propagates_an_api_error() {
    let server = FakeLangfuse::start(vec![Answer::Respond {
        status: 401,
        body: r#"{"message":"Unauthorized"}"#.to_owned(),
    }])
    .await;
    let tasks = TaskTracker::new();
    let clock = FakeClock(Arc::new(Mutex::new(Instant::now())));
    let client = client_for(&server, TTL, &clock, &tasks);
    let error = api_error(client.project_id().await.expect_err("an api error"));
    assert_eq!(error.status_code, 401);
    let expected: Json =
        serde_json::from_str(r#"{"message":"Unauthorized"}"#).expect("the fixture");
    assert_eq!(error.body, expected);
}

#[tokio::test]
async fn project_id_propagates_a_transport_error() {
    let server = FakeLangfuse::start(vec![Answer::Close]).await;
    let tasks = TaskTracker::new();
    let clock = FakeClock(Arc::new(Mutex::new(Instant::now())));
    let client = client_for(&server, TTL, &clock, &tasks);
    let error = client.project_id().await.expect_err("a transport error");
    assert!(
        matches!(
            &error,
            LangfuseRestError::Request(RequestFailure::Transport(_))
        ),
        "{error:?}"
    );
}

#[tokio::test]
async fn project_id_does_not_retry_a_server_error() {
    let server = FakeLangfuse::start(vec![Answer::Respond {
        status: 500,
        body: r#"{"message":"boom"}"#.to_owned(),
    }])
    .await;
    let tasks = TaskTracker::new();
    let clock = FakeClock(Arc::new(Mutex::new(Instant::now())));
    let client = client_for(&server, TTL, &clock, &tasks);
    let error = api_error(client.project_id().await.expect_err("an api error"));
    assert_eq!(error.status_code, 500);
    assert_eq!(server.request_count(), 1);
}

#[tokio::test]
async fn project_id_rejects_a_project_without_the_declared_fields() {
    let server = FakeLangfuse::start(vec![Answer::Respond {
        status: 200,
        body: r#"{"data":[{"id":"only-an-id"}]}"#.to_owned(),
    }])
    .await;
    let tasks = TaskTracker::new();
    let clock = FakeClock(Arc::new(Mutex::new(Instant::now())));
    let client = client_for(&server, TTL, &clock, &tasks);
    let error = client.project_id().await.expect_err("a decode error");
    assert!(matches!(error, LangfuseRestError::Decode { .. }));
}
