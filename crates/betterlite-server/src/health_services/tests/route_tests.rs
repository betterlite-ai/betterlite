use axum::http::StatusCode;
use rstest::rstest;

use crate::health_services::service_name::unknown_service_message;

use super::support::{Answer, FakeChecks, World, capture_logs, master_context};

#[tokio::test(start_paused = true)]
async fn route_test_answers_the_request_route() {
    let world = World::default();
    let reply = world.get("/test", Some(&master_context())).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.text(), r#"{"route":"/test"}"#);
}

#[tokio::test(start_paused = true)]
async fn route_test_ignores_the_query_string() {
    let world = World::default();
    let reply = world.get("/test?verbose=1", Some(&master_context())).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.text(), r#"{"route":"/test"}"#);
}

#[tokio::test(start_paused = true)]
async fn route_missing_service_gets_the_validation_422() {
    let world = World::default();
    let reply = world.get("/health/services", Some(&master_context())).await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        reply.text(),
        r#"{"detail":[{"type":"missing","loc":["query","service"],"msg":"Field required"}]}"#
    );
}

#[tokio::test(start_paused = true)]
async fn route_empty_service_gets_the_unknown_name_400() {
    let world = World::default();
    let reply = world
        .get("/health/services?service=", Some(&master_context()))
        .await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST);
    let error = reply.error();
    assert_eq!(error.error.kind, "auth_error");
    assert_eq!(
        error.error.message,
        format!("{{'error': '{}'}}", unknown_service_message(""))
    );
}

#[tokio::test(start_paused = true)]
async fn route_the_last_repeated_service_wins() {
    let world = World::default();
    let reply = world
        .get(
            "/health/services?service=nope&service=datadog",
            Some(&master_context()),
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK);
    let health = reply.health();
    assert_eq!(health.status, "healthy");
    assert_eq!(health.message.as_deref(), Some("Datadog is healthy"));
}

#[tokio::test(start_paused = true)]
async fn route_a_later_unknown_service_wins() {
    let world = World::default();
    let reply = world
        .get(
            "/health/services?service=datadog&service=nope",
            Some(&master_context()),
        )
        .await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST);
}

#[rstest]
#[case::otel("otel")]
#[case::custom_callback_api("custom_callback_api")]
#[case::langsmith("langsmith")]
#[case::langfuse_otel("langfuse_otel")]
#[case::datadog_llm_observability("datadog_llm_observability")]
#[case::signoz("signoz")]
#[tokio::test(start_paused = true)]
async fn route_a_name_without_a_branch_answers_null(#[case] service: &str) {
    let world = World::default();
    let reply = world
        .get(
            &format!("/health/services?service={service}"),
            Some(&master_context()),
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.text(), "null");
}

#[tokio::test(start_paused = true)]
async fn route_a_missing_caller_gets_the_unauthenticated_401() {
    let world = World::default();
    let reply = world.get("/health/services?service=datadog", None).await;
    assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
    let error = reply.error();
    assert_eq!(error.error.message, "the API key is not valid");
    assert_eq!(error.error.kind, "auth_error");
    assert_eq!(error.error.param.as_deref(), Some("None"));
    assert_eq!(error.error.code, "401");
}

#[tokio::test(start_paused = true)]
async fn route_a_failed_check_logs_the_error_line_and_the_debug_traceback() {
    let (logs, _guard) = capture_logs(tracing::Level::DEBUG);
    let world = World::default()
        .with_checks(FakeChecks::default().with_datadog(Answer::Failed("datadog logger failed")));
    let reply = world
        .get("/health/services?service=datadog", Some(&master_context()))
        .await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    let output = logs.contents();
    assert!(
        output.contains(
            "litellm.proxy.proxy_server.health_services_endpoint(): Exception occured - datadog logger failed"
        ),
        "{output}"
    );
    assert!(
        output.contains("Test(ServiceTestError { message:"),
        "{output}"
    );
}
