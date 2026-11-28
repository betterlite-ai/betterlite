#![cfg(test)]

mod support;

use std::sync::Arc;

use axum::http::StatusCode;
use betterlite_auth::auth_identity::roles::Role;
use betterlite_db::gateway_requests::GatewayRequestActivityResponse;
use betterlite_db::gateway_requests::GatewayRequestBreakdownEntry;
use betterlite_db::gateway_requests::GatewayRequestDailyEntry;
use betterlite_db::postgres_store::errors::DbError;
use betterlite_error::proxy_error_types::common::CommonProxyErrors;
use rstest::rstest;

use crate::auth::auth_key_lookup::tests::master_caller;
use crate::management::gateway_request_activity::auth::FORBIDDEN;
use crate::management::gateway_request_activity::error::DB_UNAVAILABLE;
use crate::management::gateway_request_activity::store::GatewayActivityRow;
use crate::management::gateway_request_activity::store::GatewayActivityStore;

use self::support::DetailBody;
use self::support::ErrorEnvelope;
use self::support::FakeStore;
use self::support::InternalEnvelope;
use self::support::Reply;
use self::support::admin;
use self::support::app;
use self::support::cached_plan_error;
use self::support::caller;
use self::support::data_error;
use self::support::gateway_database;
use self::support::get;
use self::support::postgres_store;
use self::support::row;

const ACTIVITY: &str = "/gateway/daily/activity";
const DB_ERROR: &str = "DB not connected. This endpoint needs a database; set DATABASE_URL to a PostgreSQL connection string (postgresql://...) to enable it. See https://docs.litellm.ai/docs/proxy/virtual_keys";

fn three_rows() -> Vec<GatewayActivityRow> {
    vec![
        row("2026-01-01", "api", "/v1/chat", 2, 1),
        row("2026-01-01", "api", "/v1/embed", 5, 0),
        row("2026-01-02", "api", "/v1/chat", 1, 3),
    ]
}

async fn answering(rows: Vec<GatewayActivityRow>, uri: &str) -> (Arc<FakeStore>, Reply) {
    let store = Arc::new(FakeStore::answering(rows));
    let reply = get(app(Some(FakeStore::erased(&store))), Some(&admin()), uri).await;
    (store, reply)
}

#[rstest]
#[case::master_key_subject("master_key", Vec::new(), StatusCode::FORBIDDEN)]
#[case::proxy_admin("user", vec![Role::PlatformAdmin], StatusCode::OK)]
#[case::proxy_admin_view_only("user", vec![Role::PlatformViewer], StatusCode::OK)]
#[case::admin_and_viewer("user", vec![Role::PlatformAdmin, Role::PlatformViewer], StatusCode::OK)]
#[case::org_admin("user", vec![Role::OrgAdmin], StatusCode::FORBIDDEN)]
#[case::org_viewer("user", vec![Role::OrgViewer], StatusCode::FORBIDDEN)]
#[case::team_admin("user", vec![Role::TeamAdmin], StatusCode::FORBIDDEN)]
#[case::team_member("user", vec![Role::TeamMember], StatusCode::FORBIDDEN)]
#[case::no_roles("user", Vec::new(), StatusCode::FORBIDDEN)]
#[tokio::test]
async fn the_auth_check_admits_only_the_proxy_admin_roles(
    #[case] subject: &str,
    #[case] roles: Vec<Role>,
    #[case] expected: StatusCode,
) {
    let store = Arc::new(FakeStore::answering(Vec::new()));
    let reply = get(
        app(Some(FakeStore::erased(&store))),
        Some(&caller(subject, roles)),
        ACTIVITY,
    )
    .await;
    assert_eq!(reply.status, expected);
}

#[tokio::test]
async fn a_missing_caller_gets_the_auth_error_envelope() {
    let reply = get(app(None), None, ACTIVITY).await;
    assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
    let body: ErrorEnvelope = reply.json();
    assert_eq!(body.error.message, "the API key is not valid");
    assert_eq!(body.error.error_type, "auth_error");
    assert_eq!(body.error.param, "None");
    assert_eq!(body.error.code, "401");
}

#[tokio::test]
async fn a_non_admin_caller_gets_the_auth_forbidden_detail() {
    let store = Arc::new(FakeStore::answering(Vec::new()));
    let reply = get(
        app(Some(FakeStore::erased(&store))),
        Some(&caller("user", vec![Role::OrgAdmin])),
        ACTIVITY,
    )
    .await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
    let body: DetailBody = reply.json();
    assert_eq!(body.detail, FORBIDDEN);
    assert!(store.calls().is_empty());
}

#[tokio::test]
async fn a_master_key_caller_passes_the_auth_check() {
    let reply = get(app(None), Some(&master_caller()), ACTIVITY).await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    let body: DetailBody = reply.json();
    assert_eq!(body.detail, DB_ERROR);
}

#[tokio::test]
async fn the_auth_check_runs_before_the_database_check() {
    let reply = get(
        app(None),
        Some(&caller("user", vec![Role::OrgViewer])),
        ACTIVITY,
    )
    .await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn a_missing_database_answers_after_auth_with_the_db_error_detail() {
    let reply = get(app(None), Some(&admin()), ACTIVITY).await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    let body: DetailBody = reply.json();
    assert_eq!(body.detail, DB_ERROR);
    assert_eq!(body.detail, CommonProxyErrors::DbNotConnectedError.as_str());
}

#[tokio::test]
async fn the_default_date_range_is_the_thirty_days_before_the_clock_date() {
    let (store, reply) = answering(Vec::new(), ACTIVITY).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(
        store.calls(),
        vec![("2026-02-13".to_owned(), "2026-03-15".to_owned())]
    );
}

#[rstest]
#[case::only_start("?start_date=2026-01-01", "2026-01-01", "2026-03-15")]
#[case::only_end("?end_date=2026-01-31", "2026-02-13", "2026-01-31")]
#[tokio::test]
async fn an_absent_date_parameter_falls_back_to_its_default(
    #[case] query: &str,
    #[case] expected_start: &str,
    #[case] expected_end: &str,
) {
    let (store, reply) = answering(Vec::new(), &format!("{ACTIVITY}{query}")).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(
        store.calls(),
        vec![(expected_start.to_owned(), expected_end.to_owned())]
    );
}

#[tokio::test]
async fn date_parameters_are_used_as_given() {
    let (store, reply) = answering(
        Vec::new(),
        &format!("{ACTIVITY}?start_date=2026-1-5&end_date=2026-1-31"),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(
        store.calls(),
        vec![("2026-1-5".to_owned(), "2026-1-31".to_owned())]
    );
}

#[tokio::test]
async fn a_repeated_date_parameter_takes_the_last_value() {
    let (store, reply) = answering(
        Vec::new(),
        &format!("{ACTIVITY}?start_date=2026-01-01&start_date=2026-02-02"),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(
        store.calls(),
        vec![("2026-02-02".to_owned(), "2026-03-15".to_owned())]
    );
}

#[tokio::test]
async fn an_empty_date_parameter_is_used_as_given() {
    let (store, reply) = answering(Vec::new(), &format!("{ACTIVITY}?start_date=&end_date=")).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(store.calls(), vec![(String::new(), String::new())]);
}

#[tokio::test]
async fn unknown_query_parameters_are_ignored_by_the_date_parsing() {
    let (store, reply) = answering(
        Vec::new(),
        &format!("{ACTIVITY}?limit=5&start_date=2026-01-01"),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(
        store.calls(),
        vec![("2026-01-01".to_owned(), "2026-03-15".to_owned())]
    );
}

#[tokio::test]
async fn the_aggregation_folds_rows_into_the_activity_response() {
    let (_store, reply) = answering(three_rows(), ACTIVITY).await;
    assert_eq!(reply.status, StatusCode::OK);
    let body: GatewayRequestActivityResponse = reply.json();
    assert_eq!(
        body,
        GatewayRequestActivityResponse {
            total_successful_requests: 8,
            total_failed_requests: 4,
            by_date: vec![
                GatewayRequestDailyEntry {
                    date: "2026-01-01".to_owned(),
                    successful_requests: 7,
                    failed_requests: 1,
                },
                GatewayRequestDailyEntry {
                    date: "2026-01-02".to_owned(),
                    successful_requests: 1,
                    failed_requests: 3,
                },
            ],
            by_route: vec![
                GatewayRequestBreakdownEntry {
                    category: "api".to_owned(),
                    route: "/v1/embed".to_owned(),
                    successful_requests: 5,
                    failed_requests: 0,
                },
                GatewayRequestBreakdownEntry {
                    category: "api".to_owned(),
                    route: "/v1/chat".to_owned(),
                    successful_requests: 3,
                    failed_requests: 4,
                },
            ],
        }
    );
}

#[tokio::test]
async fn the_aggregation_answers_the_exact_wire_body() {
    let (_store, reply) = answering(three_rows(), ACTIVITY).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(
        reply.text(),
        concat!(
            r#"{"total_successful_requests":8,"total_failed_requests":4,"#,
            r#""by_date":[{"date":"2026-01-01","successful_requests":7,"failed_requests":1},"#,
            r#"{"date":"2026-01-02","successful_requests":1,"failed_requests":3}],"#,
            r#""by_route":[{"category":"api","route":"/v1/embed","successful_requests":5,"failed_requests":0},"#,
            r#"{"category":"api","route":"/v1/chat","successful_requests":3,"failed_requests":4}]}"#
        )
    );
}

#[tokio::test]
async fn the_aggregation_sorts_routes_by_successful_requests_descending() {
    let (_store, reply) = answering(
        vec![
            row("2026-01-01", "api", "/low", 1, 9),
            row("2026-01-01", "api", "/high", 7, 0),
            row("2026-01-01", "api", "/mid", 4, 2),
        ],
        ACTIVITY,
    )
    .await;
    let body: GatewayRequestActivityResponse = reply.json();
    let routes: Vec<&str> = body
        .by_route
        .iter()
        .map(|entry| entry.route.as_str())
        .collect();
    assert_eq!(routes, vec!["/high", "/mid", "/low"]);
}

#[tokio::test]
async fn the_aggregation_keeps_ties_in_category_and_route_order() {
    let (_store, reply) = answering(
        vec![
            row("2026-01-01", "zeta", "/a", 2, 0),
            row("2026-01-01", "api", "/b", 2, 0),
            row("2026-01-01", "api", "/a", 2, 0),
        ],
        ACTIVITY,
    )
    .await;
    let body: GatewayRequestActivityResponse = reply.json();
    let pairs: Vec<(&str, &str)> = body
        .by_route
        .iter()
        .map(|entry| (entry.category.as_str(), entry.route.as_str()))
        .collect();
    assert_eq!(pairs, vec![("api", "/a"), ("api", "/b"), ("zeta", "/a")]);
}

#[tokio::test]
async fn an_aggregation_over_no_rows_returns_empty_breakdowns() {
    let (_store, reply) = answering(Vec::new(), ACTIVITY).await;
    assert_eq!(reply.status, StatusCode::OK);
    let body: GatewayRequestActivityResponse = reply.json();
    assert_eq!(body, GatewayRequestActivityResponse::default());
}

#[tokio::test]
async fn the_aggregation_sums_dates_in_ascending_order() {
    let (_store, reply) = answering(
        vec![
            row("2026-02-01", "api", "/v1/chat", 1, 0),
            row("2025-12-31", "api", "/v1/chat", 2, 0),
            row("2026-01-15", "api", "/v1/chat", 3, 0),
        ],
        ACTIVITY,
    )
    .await;
    let body: GatewayRequestActivityResponse = reply.json();
    let dates: Vec<&str> = body
        .by_date
        .iter()
        .map(|entry| entry.date.as_str())
        .collect();
    assert_eq!(dates, vec!["2025-12-31", "2026-01-15", "2026-02-01"]);
    assert_eq!(body.total_successful_requests, 6);
}

#[tokio::test]
async fn a_failed_aggregation_reports_a_pool_timeout_as_no_db_connection() {
    let store = Arc::new(FakeStore::failing());
    let reply = get(
        app(Some(FakeStore::erased(&store))),
        Some(&admin()),
        ACTIVITY,
    )
    .await;
    assert_eq!(reply.status, StatusCode::SERVICE_UNAVAILABLE);
    let body: ErrorEnvelope = reply.json();
    assert_eq!(body.error.message, DB_UNAVAILABLE);
    assert_eq!(body.error.error_type, "no_db_connection");
    assert_eq!(body.error.param, "None");
    assert_eq!(body.error.code, "503");
    assert_eq!(store.calls().len(), 1);
}

#[tokio::test]
async fn a_failed_aggregation_reports_a_cached_plan_error_as_no_db_connection() {
    let outage = cached_plan_error().await;
    let store = Arc::new(FakeStore::failing_with(outage));
    let reply = get(
        app(Some(FakeStore::erased(&store))),
        Some(&admin()),
        ACTIVITY,
    )
    .await;
    assert_eq!(reply.status, StatusCode::SERVICE_UNAVAILABLE);
    let body: ErrorEnvelope = reply.json();
    assert_eq!(body.error.error_type, "no_db_connection");
    assert_eq!(body.error.param, "None");
    assert_eq!(body.error.code, "503");
}

#[tokio::test]
async fn a_failed_aggregation_reports_a_data_error_as_an_internal_error() {
    let rejected = data_error().await;
    let store = Arc::new(FakeStore::failing_with(rejected));
    let reply = get(
        app(Some(FakeStore::erased(&store))),
        Some(&admin()),
        ACTIVITY,
    )
    .await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    let body: InternalEnvelope = reply.json();
    assert_eq!(body.error.message, "Internal server error");
    assert_eq!(body.error.error_type, "internal_server_error");
}

#[tokio::test]
async fn a_failed_aggregation_returns_the_internal_error_envelope() {
    let store = Arc::new(FakeStore::failing_with(DbError::CreateNull));
    let reply = get(
        app(Some(FakeStore::erased(&store))),
        Some(&admin()),
        ACTIVITY,
    )
    .await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    let body: InternalEnvelope = reply.json();
    assert_eq!(body.error.message, "Internal server error");
    assert_eq!(body.error.error_type, "internal_server_error");
    assert_eq!(store.calls().len(), 1);
}

#[tokio::test]
async fn the_aggregation_reads_the_daily_gateway_requests_table() {
    let (fixture, pool) = gateway_database().await;
    pool.get()
        .await
        .expect("a pooled client")
        .batch_execute(
            r#"INSERT INTO "LiteLLM_DailyGatewayRequests" ("date", "category", "route", "successful_requests", "failed_requests", "updated_at") VALUES
            ('2026-01-01', 'api', '/v1/chat', 2, 1, CURRENT_TIMESTAMP),
            ('2026-01-02', 'api', '/v1/chat', 3, 0, CURRENT_TIMESTAMP),
            ('2025-12-31', 'api', '/v1/chat', 9, 9, CURRENT_TIMESTAMP);"#,
        )
        .await
        .expect("the seeded rows");
    let store: Arc<dyn GatewayActivityStore> = Arc::new(postgres_store(&pool));
    let reply = get(
        app(Some(store)),
        Some(&admin()),
        &format!("{ACTIVITY}?start_date=2026-01-01&end_date=2026-01-02"),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let body: GatewayRequestActivityResponse = reply.json();
    assert_eq!(body.total_successful_requests, 5);
    assert_eq!(body.total_failed_requests, 1);
    assert_eq!(body.by_date.len(), 2);
    assert_eq!(body.by_route.len(), 1);
    let breakdown: &GatewayRequestBreakdownEntry = body.by_route.first().expect("one route");
    assert_eq!(breakdown.category, "api");
    assert_eq!(breakdown.route, "/v1/chat");
    assert_eq!(breakdown.successful_requests, 5);
    assert_eq!(breakdown.failed_requests, 1);
    pool.close();
    fixture.close_async().await.expect("the fixture closes");
}
