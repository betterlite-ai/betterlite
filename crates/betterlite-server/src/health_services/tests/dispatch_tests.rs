use axum::http::StatusCode;
use rstest::rstest;

use super::support::{
    Answer, FakeCallbacks, FakeChecks, FakeMock, World, assert_mock_request, master_context,
};

#[rstest]
#[case::openmeter("openmeter")]
#[case::braintrust("braintrust")]
#[case::generic_api("generic_api")]
#[tokio::test(start_paused = true)]
async fn dispatch_forced_mock_services_make_the_mock_request(#[case] service: &str) {
    let world = World::default();
    let reply = world
        .get(
            &format!("/health/services?service={service}"),
            Some(&master_context()),
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK);
    let success = reply.success();
    assert_eq!(success.status, "success");
    assert_eq!(
        success.message,
        format!("Mock LLM request made - check {service}.")
    );
    let requests = world.mock.requests();
    assert_eq!(requests.len(), 1);
    assert_mock_request(&requests[0]);
}

#[tokio::test(start_paused = true)]
async fn dispatch_a_listed_plain_callback_wins_over_the_vendor_branch() {
    let world = World::default().with_callbacks(FakeCallbacks::plain(&["datadog"]));
    let reply = world
        .get("/health/services?service=datadog", Some(&master_context()))
        .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(
        reply.success().message,
        "Mock LLM request made - check datadog."
    );
    let requests = world.mock.requests();
    assert_eq!(requests.len(), 1);
    assert_mock_request(&requests[0]);
}

#[tokio::test(start_paused = true)]
async fn dispatch_a_registered_callback_name_wins_over_the_vendor_branch() {
    let world = World::default().with_callbacks(FakeCallbacks::registered("arize"));
    let reply = world
        .get("/health/services?service=arize", Some(&master_context()))
        .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(
        reply.success().message,
        "Mock LLM request made - check arize."
    );
    assert_eq!(world.mock.requests().len(), 1);
}

#[tokio::test(start_paused = true)]
async fn dispatch_a_listed_service_without_a_branch_makes_the_mock_request() {
    let world = World::default().with_callbacks(FakeCallbacks::plain(&["otel"]));
    let reply = world
        .get("/health/services?service=otel", Some(&master_context()))
        .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(
        reply.success().message,
        "Mock LLM request made - check otel."
    );
    assert_eq!(world.mock.requests().len(), 1);
}

#[tokio::test(start_paused = true)]
async fn dispatch_langfuse_in_the_callback_list_still_checks_auth() {
    let world = World::default()
        .with_callbacks(FakeCallbacks::plain(&["langfuse"]))
        .with_checks(FakeChecks::default().with_langfuse(Some("bad key")));
    let reply = world
        .get("/health/services?service=langfuse", Some(&master_context()))
        .await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    let error = reply.error();
    assert_eq!(error.error.kind, "auth_error");
    assert_eq!(error.error.param.as_deref(), Some("None"));
    assert_eq!(error.error.code, "500");
    assert_eq!(
        error.error.message,
        "Authentication Error, langfuse auth_check failed: bad key"
    );
    assert!(world.mock.requests().is_empty());
}

#[tokio::test(start_paused = true)]
async fn dispatch_langfuse_outside_the_callback_list_checks_auth_then_mocks() {
    let world = World::default();
    let reply = world
        .get("/health/services?service=langfuse", Some(&master_context()))
        .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(
        reply.success().message,
        "Mock LLM request made - check langfuse."
    );
    let requests = world.mock.requests();
    assert_eq!(requests.len(), 1);
    assert_mock_request(&requests[0]);
}

#[tokio::test(start_paused = true)]
async fn dispatch_pointfive_in_the_callback_list_still_pings() {
    let world = World::default()
        .with_callbacks(FakeCallbacks::plain(&["pointfive"]))
        .with_checks(
            FakeChecks::default().with_pointfive(Answer::Failed("PointFive API key not set.")),
        );
    let reply = world
        .get(
            "/health/services?service=pointfive",
            Some(&master_context()),
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK);
    let health = reply.health();
    assert_eq!(health.status, "unhealthy");
    assert_eq!(
        health.message.as_deref(),
        Some("PointFive API key not set.")
    );
    assert!(world.mock.requests().is_empty());
}

#[tokio::test(start_paused = true)]
async fn dispatch_mock_failure_answers_the_authentication_error() {
    let world = World::default().with_mock(FakeMock::failing("completion exploded"));
    let reply = world
        .get(
            "/health/services?service=openmeter",
            Some(&master_context()),
        )
        .await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    let error = reply.error();
    assert_eq!(error.error.kind, "auth_error");
    assert_eq!(error.error.param.as_deref(), Some("None"));
    assert_eq!(error.error.code, "500");
    assert_eq!(
        error.error.message,
        "Authentication Error, completion exploded"
    );
}
