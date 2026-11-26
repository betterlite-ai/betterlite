use axum::http::StatusCode;
use rstest::rstest;

use crate::auth::auth_identity::AuthContext;

use super::support::{Answer, FakeChecks, World, master_context, user_context, viewer_context};

#[rstest]
#[case::healthy(Answer::Healthy, "healthy", Some("Datadog is healthy"))]
#[case::healthy_ignores_error_text(
    Answer::HealthyWithText("leftover"),
    "healthy",
    Some("Datadog is healthy")
)]
#[case::unhealthy(Answer::Unhealthy("datadog down"), "unhealthy", Some("datadog down"))]
#[tokio::test(start_paused = true)]
async fn vendor_datadog_answers_its_health_check(
    #[case] answer: Answer,
    #[case] status: &str,
    #[case] message: Option<&str>,
) {
    let world = World::default().with_checks(FakeChecks::default().with_datadog(answer));
    let reply = world
        .get("/health/services?service=datadog", Some(&master_context()))
        .await;
    assert_eq!(reply.status, StatusCode::OK);
    let health = reply.health();
    assert_eq!(health.status, status);
    assert_eq!(health.message.as_deref(), message);
}

#[rstest]
#[case::healthy(Answer::Healthy, "healthy", Some("Datadog Metrics is healthy"))]
#[case::unhealthy(Answer::Unhealthy("metrics down"), "unhealthy", Some("metrics down"))]
#[tokio::test(start_paused = true)]
async fn vendor_datadog_metrics_answers_its_health_check(
    #[case] answer: Answer,
    #[case] status: &str,
    #[case] message: Option<&str>,
) {
    let world = World::default().with_checks(FakeChecks::default().with_datadog_metrics(answer));
    let reply = world
        .get(
            "/health/services?service=datadog_metrics",
            Some(&master_context()),
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK);
    let health = reply.health();
    assert_eq!(health.status, status);
    assert_eq!(health.message.as_deref(), message);
}

#[rstest]
#[case::healthy(Answer::Healthy, "healthy", Some("Arize is healthy"))]
#[case::unhealthy(Answer::Unhealthy("arize down"), "unhealthy", Some("arize down"))]
#[tokio::test(start_paused = true)]
async fn vendor_arize_answers_its_health_check(
    #[case] answer: Answer,
    #[case] status: &str,
    #[case] message: Option<&str>,
) {
    let world = World::default().with_checks(FakeChecks::default().with_arize(answer));
    let reply = world
        .get("/health/services?service=arize", Some(&master_context()))
        .await;
    assert_eq!(reply.status, StatusCode::OK);
    let health = reply.health();
    assert_eq!(health.status, status);
    assert_eq!(health.message.as_deref(), message);
}

#[rstest]
#[case::healthy(Answer::Healthy, "healthy", Some("Galileo is healthy"))]
#[case::unhealthy(Answer::Unhealthy("galileo down"), "unhealthy", Some("galileo down"))]
#[tokio::test(start_paused = true)]
async fn vendor_galileo_answers_its_health_check(
    #[case] answer: Answer,
    #[case] status: &str,
    #[case] message: Option<&str>,
) {
    let world = World::default().with_checks(FakeChecks::default().with_galileo(answer));
    let reply = world
        .get("/health/services?service=galileo", Some(&master_context()))
        .await;
    assert_eq!(reply.status, StatusCode::OK);
    let health = reply.health();
    assert_eq!(health.status, status);
    assert_eq!(health.message.as_deref(), message);
}

#[rstest]
#[case::healthy(Answer::Healthy, "healthy", None)]
#[case::healthy_keeps_the_error_text(
    Answer::HealthyWithText("leftover"),
    "healthy",
    Some("leftover")
)]
#[case::unhealthy(Answer::Unhealthy("sqs down"), "unhealthy", Some("sqs down"))]
#[tokio::test(start_paused = true)]
async fn vendor_sqs_answers_the_error_text_even_when_healthy(
    #[case] answer: Answer,
    #[case] status: &str,
    #[case] message: Option<&str>,
) {
    let world = World::default().with_checks(FakeChecks::default().with_sqs(answer));
    let reply = world
        .get("/health/services?service=sqs", Some(&master_context()))
        .await;
    assert_eq!(reply.status, StatusCode::OK);
    let health = reply.health();
    assert_eq!(health.status, status);
    assert_eq!(health.message.as_deref(), message);
}

#[tokio::test(start_paused = true)]
async fn vendor_newrelic_requires_a_proxy_admin() {
    let world = World::default();
    let reply = world
        .get("/health/services?service=newrelic", Some(&user_context()))
        .await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
    let error = reply.error();
    assert_eq!(error.error.kind, "auth_error");
    assert_eq!(error.error.param.as_deref(), Some("None"));
    assert_eq!(error.error.code, "403");
    assert_eq!(
        error.error.message,
        "{'error': 'Only proxy admins can trigger the New Relic test event.'}"
    );
}

#[rstest]
#[case::full_admin(master_context())]
#[case::view_only_admin(viewer_context())]
#[tokio::test(start_paused = true)]
async fn vendor_newrelic_admins_get_the_test_event_message(#[case] caller: AuthContext) {
    let world = World::default();
    let reply = world
        .get("/health/services?service=newrelic", Some(&caller))
        .await;
    assert_eq!(reply.status, StatusCode::OK);
    let health = reply.health();
    assert_eq!(health.status, "healthy");
    assert_eq!(
        health.message.as_deref(),
        Some("New Relic is healthy \u{2014} test event sent")
    );
}

#[tokio::test(start_paused = true)]
async fn vendor_newrelic_unhealthy_reports_the_error_text() {
    let world = World::default().with_checks(
        FakeChecks::default().with_newrelic(Answer::Unhealthy("newrelic key rejected")),
    );
    let reply = world
        .get("/health/services?service=newrelic", Some(&master_context()))
        .await;
    assert_eq!(reply.status, StatusCode::OK);
    let health = reply.health();
    assert_eq!(health.status, "unhealthy");
    assert_eq!(health.message.as_deref(), Some("newrelic key rejected"));
}

#[tokio::test(start_paused = true)]
async fn vendor_pointfive_requires_a_proxy_admin() {
    let world = World::default();
    let reply = world
        .get("/health/services?service=pointfive", Some(&user_context()))
        .await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
    let error = reply.error();
    assert_eq!(error.error.code, "403");
    assert_eq!(
        error.error.message,
        "{'error': 'Only proxy admins can trigger the PointFive liveness ping.'}"
    );
}

#[rstest]
#[case::missing_key(
    Answer::Failed("PointFive API key not set."),
    "unhealthy",
    Some("PointFive API key not set.")
)]
#[case::unhealthy(Answer::Unhealthy("ping failed"), "unhealthy", Some("ping failed"))]
#[case::empty_error_reads_healthy(Answer::Unhealthy(""), "unhealthy", Some("PointFive is healthy"))]
#[case::healthy(Answer::Healthy, "healthy", Some("PointFive is healthy"))]
#[case::healthy_ignores_error_text(
    Answer::HealthyWithText("leftover"),
    "healthy",
    Some("PointFive is healthy")
)]
#[tokio::test(start_paused = true)]
async fn vendor_pointfive_answers_the_liveness_ping(
    #[case] answer: Answer,
    #[case] status: &str,
    #[case] message: Option<&str>,
) {
    let world = World::default().with_checks(FakeChecks::default().with_pointfive(answer));
    let reply = world
        .get(
            "/health/services?service=pointfive",
            Some(&master_context()),
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK);
    let health = reply.health();
    assert_eq!(health.status, status);
    assert_eq!(health.message.as_deref(), message);
}

#[tokio::test(start_paused = true)]
async fn vendor_pointfive_view_only_admin_passes() {
    let world = World::default();
    let reply = world
        .get(
            "/health/services?service=pointfive",
            Some(&viewer_context()),
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK);
    let health = reply.health();
    assert_eq!(health.status, "healthy");
    assert_eq!(health.message.as_deref(), Some("PointFive is healthy"));
}

#[tokio::test(start_paused = true)]
async fn vendor_check_failure_answers_the_authentication_error() {
    let world = World::default()
        .with_checks(FakeChecks::default().with_datadog(Answer::Failed("datadog logger failed")));
    let reply = world
        .get("/health/services?service=datadog", Some(&master_context()))
        .await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    let error = reply.error();
    assert_eq!(error.error.kind, "auth_error");
    assert_eq!(error.error.code, "500");
    assert_eq!(
        error.error.message,
        "Authentication Error, datadog logger failed"
    );
}
