#![cfg(test)]

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use axum::Router;
use axum::extract::{Request, State};
use axum::http::{Method, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::Response;
use axum::routing::get;
use betterlite_config::sections::admission_control::{Limits, Settings as AdmissionSettings};
use betterlite_db::database_availability::policy::AllowRequestsOnOutage;
use betterlite_db::database_faults::ErrorType;
use betterlite_prometheus::collector_registry::CollectorRegistry;
use rstest::rstest;
use secrecy::SecretString;
use serde::Deserialize;
use tokio::time::sleep;

use super::AdmissionCounters;
use crate::admission_control::in_flight::InFlightRequests;
use crate::admission_control::{AdmissionControl, AdmissionStack};
use crate::auth::authenticate;
use crate::auth::failures::stage_failure_response;
use crate::auth::route_match::stage;
use crate::health_readiness::callback_listing::CallbackNames;
use crate::health_readiness::readiness_routes::AdmissionCounter;
use crate::health_readiness::shutdown_state::InFlightCounter;
use crate::health_readiness::tests::support::{
    FakeAdmission, FakeCallbacks, FakeInFlight, FakeProbe, FakeStall, ProbeAnswer, World,
    header_text, parse, send, send_unread,
};

#[derive(Debug, Deserialize)]
struct StatusBody {
    status: String,
}

#[derive(Debug, Deserialize)]
struct PublicBody {
    status: String,
    db: String,
}

#[derive(Debug, Deserialize)]
struct PublicDetailedBody {
    status: String,
    db: String,
    litellm_version: String,
}

#[derive(Debug, Deserialize)]
struct DetailBody {
    detail: String,
}

#[derive(Debug, Deserialize)]
struct BacklogBody {
    in_flight_requests: u64,
    admitted_requests: u64,
    queued_requests: u64,
    rejected_requests: u64,
}

#[derive(Debug, Deserialize)]
struct DrainBody {
    status: String,
    drained_requests: u64,
}

#[derive(Debug, Deserialize)]
struct LicenseBody {
    has_license: bool,
    license_type: String,
    expiration_date: Option<String>,
    allowed_features: Vec<String>,
    limits: LicenseLimits,
}

#[derive(Debug, Deserialize)]
struct LicenseLimits {
    max_users: Option<u32>,
    max_teams: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct ListingBody {
    alerting: String,
    #[serde(rename = "litellm.callbacks")]
    callbacks: Vec<String>,
    #[serde(rename = "litellm.input_callback")]
    input_callback: Vec<String>,
    #[serde(rename = "litellm.failure_callback")]
    failure_callback: Vec<String>,
    #[serde(rename = "litellm.success_callback")]
    success_callback: Vec<String>,
    #[serde(rename = "litellm._async_success_callback")]
    async_success_callback: Vec<String>,
    #[serde(rename = "litellm._async_failure_callback")]
    async_failure_callback: Vec<String>,
    #[serde(rename = "litellm._async_input_callback")]
    async_input_callback: Vec<String>,
    all_litellm_callbacks: Vec<String>,
    num_callbacks: usize,
    num_alerting: usize,
    #[serde(rename = "litellm.request_timeout")]
    request_timeout: f64,
}

#[tokio::test]
async fn public_readiness_reports_shutting_down_first() {
    let mut world = World::new();
    world.settings.allow_public_details = true;
    world.settings.drain.enabled = true;
    world.probe = Arc::new(FakeProbe::healthy());
    let router = world.router(AllowRequestsOnOutage::Unsupported);
    let (drain_status, _, _) = send(&router, Method::GET, "/health/drain", None).await;
    assert_eq!(drain_status, StatusCode::OK);
    let (status, _, body) = send(&router, Method::GET, "/health/readiness", None).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    let parsed: StatusBody = parse(&body);
    assert_eq!(parsed.status, "shutting_down");
}

#[tokio::test]
async fn details_route_ignores_the_shutdown_flag() {
    let mut world = World::new();
    world.settings.drain.enabled = true;
    world.probe = Arc::new(FakeProbe::healthy());
    let router = world.router(AllowRequestsOnOutage::Unsupported);
    let (drain_status, _, _) = send(&router, Method::GET, "/health/drain", None).await;
    assert_eq!(drain_status, StatusCode::OK);
    let (status, _, body) = send(&router, Method::GET, "/health/readiness/details", None).await;
    assert_eq!(status, StatusCode::OK);
    let parsed: PublicDetailedBody = parse(&body);
    assert_eq!(parsed.status, "healthy");
}

#[tokio::test]
async fn public_readiness_serves_the_details_when_allowed() {
    let mut world = World::new();
    world.settings.allow_public_details = true;
    world.probe = Arc::new(FakeProbe::healthy());
    let router = world.router(AllowRequestsOnOutage::Unsupported);
    let (status, _, body) = send(&router, Method::GET, "/health/readiness", None).await;
    assert_eq!(status, StatusCode::OK);
    let parsed: PublicDetailedBody = parse(&body);
    assert_eq!(parsed.status, "healthy");
    assert_eq!(parsed.db, "connected");
    assert_eq!(parsed.litellm_version, "betterlite-test");
}

#[tokio::test]
async fn public_readiness_reports_healthy_and_not_connected_without_a_database() {
    let router = World::new().router(AllowRequestsOnOutage::Unsupported);
    let (status, _, body) = send(&router, Method::GET, "/health/readiness", None).await;
    assert_eq!(status, StatusCode::OK);
    let parsed: PublicBody = parse(&body);
    assert_eq!(parsed.status, "healthy");
    assert_eq!(parsed.db, "Not connected");
}

#[tokio::test]
async fn public_readiness_answers_503_for_an_unreachable_database() {
    let mut world = World::new();
    world.probe = Arc::new(FakeProbe::scripted(vec![ProbeAnswer::Fail(
        ErrorType::Data,
    )]));
    let router = world.router(AllowRequestsOnOutage::Unsupported);
    let (status, _, body) = send(&router, Method::GET, "/health/readiness", None).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    let parsed: PublicBody = parse(&body);
    assert_eq!(parsed.db, "disconnected");
}

#[tokio::test]
async fn public_readiness_keeps_200_when_requests_are_allowed_on_outage() {
    let mut world = World::new();
    world.probe = Arc::new(FakeProbe::scripted(vec![ProbeAnswer::Fail(
        ErrorType::Data,
    )]));
    let router = world.router(AllowRequestsOnOutage::Boolean(true));
    let (status, _, body) = send(&router, Method::GET, "/health/readiness", None).await;
    assert_eq!(status, StatusCode::OK);
    let parsed: PublicBody = parse(&body);
    assert_eq!(parsed.db, "disconnected");
}

#[tokio::test]
async fn public_readiness_reports_a_stalled_database_as_unavailable() {
    let mut world = World::new();
    world.probe = Arc::new(FakeProbe::healthy());
    world.stall = Arc::new(FakeStall { stalled: true });
    let router = world.router(AllowRequestsOnOutage::Unsupported);
    let (status, _, body) = send(&router, Method::GET, "/health/readiness", None).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    let parsed: PublicBody = parse(&body);
    assert_eq!(parsed.db, "stalled");
}

#[tokio::test]
async fn backlog_returns_in_flight_and_admission_counters() {
    let mut world = World::new();
    world.in_flight = Arc::new(FakeInFlight::new(7));
    world.admission = Arc::new(FakeAdmission {
        stats: AdmissionCounters {
            admitted: 11,
            queued: 2,
            rejected: 3,
        },
    });
    let router = world.router(AllowRequestsOnOutage::Unsupported);
    let (status, _, body) = send(&router, Method::GET, "/health/backlog", None).await;
    assert_eq!(status, StatusCode::OK);
    let parsed: BacklogBody = parse(&body);
    assert_eq!(parsed.in_flight_requests, 7);
    assert_eq!(parsed.admitted_requests, 11);
    assert_eq!(parsed.queued_requests, 2);
    assert_eq!(parsed.rejected_requests, 3);
}

#[tokio::test]
async fn drain_route_is_404_when_disabled() {
    let router = World::new().router(AllowRequestsOnOutage::Unsupported);
    let (status, _, body) = send(&router, Method::GET, "/health/drain", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let parsed: DetailBody = parse(&body);
    assert_eq!(parsed.detail, "Not Found");
}

#[rstest]
#[case::missing(None)]
#[case::wrong(Some(("x-drain-token", "wrong-secret")))]
#[tokio::test]
async fn drain_route_rejects_a_missing_or_wrong_token(#[case] header_value: Option<(&str, &str)>) {
    let mut world = World::new();
    world.settings.drain.enabled = true;
    world.settings.drain.token = Some(SecretString::from("pre-stop-secret".to_owned()));
    let router = world.router(AllowRequestsOnOutage::Unsupported);
    let (status, _, body) = send(&router, Method::GET, "/health/drain", header_value).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let parsed: DetailBody = parse(&body);
    assert_eq!(parsed.detail, "Invalid or missing X-Drain-Token");
    let (probe_status, _, _) = send(&router, Method::GET, "/health/readiness", None).await;
    assert_eq!(probe_status, StatusCode::OK);
}

#[tokio::test(start_paused = true)]
async fn drain_route_accepts_the_configured_token_and_returns_the_drained_count() {
    let mut world = World::new();
    world.settings.drain.enabled = true;
    world.settings.drain.token = Some(SecretString::from("pre-stop-secret".to_owned()));
    world.in_flight = Arc::new(FakeInFlight::new(3));
    let counter = Arc::clone(&world.in_flight);
    let router = world.router(AllowRequestsOnOutage::Unsupported);
    let (reply, ()) = tokio::join!(
        send(
            &router,
            Method::GET,
            "/health/drain",
            Some(("x-drain-token", "pre-stop-secret")),
        ),
        async move {
            sleep(Duration::from_millis(100)).await;
            counter.count.store(1, Ordering::Release);
        }
    );
    let (status, _, body) = reply;
    assert_eq!(status, StatusCode::OK);
    let parsed: DrainBody = parse(&body);
    assert_eq!(parsed.status, "drained");
    assert_eq!(parsed.drained_requests, 2);
    let (readiness, _, _) = send(&router, Method::GET, "/health/readiness", None).await;
    assert_eq!(readiness, StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test(start_paused = true)]
async fn drain_route_needs_no_token_when_none_is_configured() {
    let mut world = World::new();
    world.settings.drain.enabled = true;
    world.in_flight = Arc::new(FakeInFlight::new(1));
    let router = world.router(AllowRequestsOnOutage::Unsupported);
    let (status, _, body) = send(&router, Method::GET, "/health/drain", None).await;
    assert_eq!(status, StatusCode::OK);
    let parsed: DrainBody = parse(&body);
    assert_eq!(parsed.drained_requests, 0);
}

#[rstest]
#[case::readiness("/health/readiness")]
#[case::liveliness("/health/liveliness")]
#[case::liveness("/health/liveness")]
#[tokio::test]
async fn options_routes_answer_200_with_the_allow_headers(#[case] path: &str) {
    let router = World::new().router(AllowRequestsOnOutage::Unsupported);
    let (status, headers, body) = send(&router, Method::OPTIONS, path, None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.is_empty());
    assert_eq!(header_text(&headers, &header::ALLOW), "GET, OPTIONS");
    assert_eq!(
        header_text(&headers, &header::ACCESS_CONTROL_ALLOW_METHODS),
        "GET, OPTIONS"
    );
    assert_eq!(
        header_text(&headers, &header::ACCESS_CONTROL_ALLOW_HEADERS),
        "*"
    );
}

#[tokio::test]
async fn license_route_answers_the_community_body() {
    let router = World::new().router(AllowRequestsOnOutage::Unsupported);
    let (status, _, body) = send(&router, Method::GET, "/health/license", None).await;
    assert_eq!(status, StatusCode::OK);
    let parsed: LicenseBody = parse(&body);
    assert!(!parsed.has_license);
    assert_eq!(parsed.license_type, "community");
    assert_eq!(parsed.expiration_date, None);
    assert!(parsed.allowed_features.is_empty());
    assert_eq!(parsed.limits.max_users, None);
    assert_eq!(parsed.limits.max_teams, None);
}

fn listing(names: CallbackNames) -> World {
    let mut world = World::new();
    world.callbacks = Arc::new(FakeCallbacks { names });
    world
}

fn names() -> CallbackNames {
    CallbackNames {
        callbacks: vec!["callbacks".to_owned()],
        input: vec!["input".to_owned()],
        failure: vec!["failure".to_owned()],
        success: vec!["success".to_owned()],
        async_success: vec!["async_success".to_owned()],
        async_failure: vec!["async_failure".to_owned()],
        async_input: vec!["async_input".to_owned()],
    }
}

#[tokio::test]
async fn callbacks_route_lists_the_seven_lists_and_counts() {
    let mut world = listing(names());
    world.settings.alerting = Some(vec!["slack".to_owned(), "email".to_owned()]);
    world.settings.request_timeout = 1200.0;
    let router = world.router(AllowRequestsOnOutage::Unsupported);
    let (status, _, body) = send(&router, Method::GET, "/settings", None).await;
    assert_eq!(status, StatusCode::OK);
    let parsed: ListingBody = parse(&body);
    assert_eq!(parsed.alerting, "['slack', 'email']");
    assert_eq!(parsed.callbacks, vec!["callbacks".to_owned()]);
    assert_eq!(parsed.input_callback, vec!["input".to_owned()]);
    assert_eq!(parsed.failure_callback, vec!["failure".to_owned()]);
    assert_eq!(parsed.success_callback, vec!["success".to_owned()]);
    assert_eq!(
        parsed.async_success_callback,
        vec!["async_success".to_owned()]
    );
    assert_eq!(
        parsed.async_failure_callback,
        vec!["async_failure".to_owned()]
    );
    assert_eq!(parsed.async_input_callback, vec!["async_input".to_owned()]);
    assert_eq!(
        parsed.all_litellm_callbacks,
        vec![
            "callbacks".to_owned(),
            "input".to_owned(),
            "failure".to_owned(),
            "success".to_owned(),
            "async_success".to_owned(),
            "async_failure".to_owned(),
            "async_input".to_owned(),
        ]
    );
    assert_eq!(parsed.num_callbacks, 7);
    assert_eq!(parsed.num_alerting, 2);
    assert_eq!(parsed.request_timeout, 1200.0);
}

#[tokio::test]
async fn callbacks_active_callbacks_answers_the_same_listing() {
    let world = listing(names());
    let router = world.router(AllowRequestsOnOutage::Unsupported);
    let (settings_status, _, settings_body) = send(&router, Method::GET, "/settings", None).await;
    let (active_status, _, active_body) =
        send(&router, Method::GET, "/active/callbacks", None).await;
    assert_eq!(settings_status, StatusCode::OK);
    assert_eq!(active_status, StatusCode::OK);
    assert_eq!(settings_body, active_body);
}

#[tokio::test]
async fn settings_alerting_is_none_when_absent() {
    let router = listing(names()).router(AllowRequestsOnOutage::Unsupported);
    let (status, _, body) = send(&router, Method::GET, "/settings", None).await;
    assert_eq!(status, StatusCode::OK);
    let parsed: ListingBody = parse(&body);
    assert_eq!(parsed.alerting, "None");
    assert_eq!(parsed.num_alerting, 0);
    assert_eq!(parsed.request_timeout, 6000.0);
}

#[tokio::test]
async fn settings_alerting_renders_an_empty_list() {
    let mut world = listing(names());
    world.settings.alerting = Some(Vec::new());
    let router = world.router(AllowRequestsOnOutage::Unsupported);
    let (status, _, body) = send(&router, Method::GET, "/settings", None).await;
    assert_eq!(status, StatusCode::OK);
    let parsed: ListingBody = parse(&body);
    assert_eq!(parsed.alerting, "[]");
    assert_eq!(parsed.num_alerting, 0);
}

async fn reached() -> &'static str {
    "reached"
}

async fn auth_layer(
    State(master_key): State<Option<SecretString>>,
    request: Request,
    next: Next,
) -> Response {
    authenticate(State(master_key), request, next, stage_failure_response).await
}

fn guarded_router() -> Router {
    Router::new()
        .route("/health/readiness", get(reached).options(reached))
        .route("/health/liveliness", get(reached).options(reached))
        .route("/health/liveness", get(reached).options(reached))
        .route("/health/drain", get(reached))
        .route("/health/readiness/details", get(reached))
        .route("/health/backlog", get(reached))
        .route("/other/path", get(reached))
        .layer(middleware::from_fn_with_state(
            Some(SecretString::from("sk-master".to_owned())),
            auth_layer,
        ))
}

#[rstest]
#[case::readiness("/health/readiness")]
#[case::liveliness("/health/liveliness")]
#[case::liveness("/health/liveness")]
#[case::drain("/health/drain")]
#[tokio::test]
async fn public_health_paths_skip_master_key_auth(#[case] path: &str) {
    let router = guarded_router();
    let (status, _, body) = send(&router, Method::GET, path, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(String::from_utf8(body).unwrap(), "reached");
}

#[rstest]
#[case::readiness("/health/readiness")]
#[case::liveliness("/health/liveliness")]
#[case::liveness("/health/liveness")]
#[tokio::test]
async fn options_health_paths_skip_master_key_auth(#[case] path: &str) {
    let router = guarded_router();
    let (status, _, body) = send(&router, Method::OPTIONS, path, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(String::from_utf8(body).unwrap(), "reached");
}

#[tokio::test]
async fn backlog_route_still_needs_the_master_key() {
    let router = guarded_router();
    let (status, _, body) = send(&router, Method::GET, "/health/backlog", None).await;
    assert_ne!(status, StatusCode::OK);
    assert_ne!(String::from_utf8(body).unwrap(), "reached");
}

#[tokio::test]
async fn details_route_still_needs_the_master_key() {
    let router = guarded_router();
    let (status, _, body) = send(&router, Method::GET, "/health/readiness/details", None).await;
    assert_ne!(status, StatusCode::OK);
    assert_ne!(String::from_utf8(body).unwrap(), "reached");
}

fn admitted_router() -> Router {
    Router::new()
        .route("/health/readiness", get(reached).options(reached))
        .route("/health/liveliness", get(reached).options(reached))
        .route("/health/liveness", get(reached).options(reached))
        .route("/health/drain", get(reached))
        .route("/health/readiness/details", get(reached))
        .route("/health/backlog", get(reached))
        .route("/health/license", get(reached))
        .route("/settings", get(reached))
        .route("/active/callbacks", get(reached))
        .route("/unlisted/path", get(reached))
        .layer(middleware::from_fn(stage))
}

#[rstest]
#[case::readiness("/health/readiness")]
#[case::liveliness("/health/liveliness")]
#[case::liveness("/health/liveness")]
#[case::drain("/health/drain")]
#[tokio::test]
async fn public_health_paths_skip_route_admission(#[case] path: &str) {
    let router = admitted_router();
    let (status, _, body) = send(&router, Method::GET, path, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(String::from_utf8(body).unwrap(), "reached");
}

#[rstest]
#[case::readiness("/health/readiness")]
#[case::liveliness("/health/liveliness")]
#[case::liveness("/health/liveness")]
#[tokio::test]
async fn options_health_paths_skip_route_admission(#[case] path: &str) {
    let router = admitted_router();
    let (status, _, body) = send(&router, Method::OPTIONS, path, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(String::from_utf8(body).unwrap(), "reached");
}

#[tokio::test]
async fn backlog_route_passes_route_admission() {
    let router = admitted_router();
    let (status, _, body) = send(&router, Method::GET, "/health/backlog", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(String::from_utf8(body).unwrap(), "reached");
}

#[tokio::test]
async fn details_route_passes_route_admission() {
    let router = admitted_router();
    let (status, _, body) = send(&router, Method::GET, "/health/readiness/details", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(String::from_utf8(body).unwrap(), "reached");
}

#[tokio::test]
async fn license_route_passes_route_admission() {
    let router = admitted_router();
    let (status, _, body) = send(&router, Method::GET, "/health/license", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(String::from_utf8(body).unwrap(), "reached");
}

#[rstest]
#[case::settings("/settings")]
#[case::active_callbacks("/active/callbacks")]
#[tokio::test]
async fn callbacks_routes_pass_route_admission(#[case] path: &str) {
    let router = admitted_router();
    let (status, _, body) = send(&router, Method::GET, path, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(String::from_utf8(body).unwrap(), "reached");
}

#[tokio::test]
async fn public_unlisted_route_is_not_admitted() {
    let router = admitted_router();
    let (status, _, body) = send(&router, Method::GET, "/unlisted/path", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let parsed: DetailBody = parse(&body);
    assert_eq!(parsed.detail, "Not Found");
}

fn admission_stack(limits: Option<Limits>) -> AdmissionStack {
    let registry = Arc::new(CollectorRegistry::default());
    let settings = AdmissionSettings { limits };
    AdmissionStack {
        admission: Arc::new(AdmissionControl::from_settings(
            &settings,
            Arc::clone(&registry),
        )),
        in_flight: Arc::new(InFlightRequests::new(registry)),
    }
}

#[tokio::test]
async fn backlog_counts_the_real_admission_stats() {
    let stack = admission_stack(Some(Limits {
        max_in_flight_requests: 0,
        max_queued_requests: 0,
        queue_timeout: Duration::ZERO,
    }));
    let router = stack.wrap(Router::new().route("/probe", get(reached)));
    let (status, _, _) = send(&router, Method::GET, "/probe", None).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    let counters = stack.admission.counters();
    assert_eq!(counters.admitted, 0);
    assert_eq!(counters.queued, 0);
    assert_eq!(counters.rejected, 1);
}

#[tokio::test]
async fn shutdown_counts_real_in_flight_requests() {
    let stack = admission_stack(None);
    let router = stack.wrap(Router::new().route("/probe", get(reached)));
    let response = send_unread(&router, Method::GET, "/probe").await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(stack.in_flight.in_flight_requests(), 1);
    drop(response);
    assert_eq!(stack.in_flight.in_flight_requests(), 0);
}
