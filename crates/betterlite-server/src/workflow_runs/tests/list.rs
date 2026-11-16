use std::sync::{Arc, Mutex};

use axum::http::{Method, StatusCode};
use betterlite_auth::auth_identity::records::Principal;
use betterlite_auth::auth_identity::roles::Role;
use betterlite_error::proxy_error_types::common::CommonProxyErrors;
use rstest::rstest;

use super::support::{
    DetailReply, FakeStore, ListReply, LocReply, ValidationReply, call, master, member, principal,
    proxy_admin, viewer,
};
use crate::workflow_runs::models::StatusFilter;

fn seeded() -> Arc<FakeStore> {
    let store = Arc::new(FakeStore::default());
    store.seed_run(Some("hash-1"), "review", "running");
    store.seed_run(Some("hash-2"), "review", "paused");
    store.seed_run(Some("hash-1"), "build", "completed");
    store.seed_run(None, "review", "pending");
    store
}

fn run_ids(body: &str) -> Vec<String> {
    let parsed: ListReply = serde_json::from_str(body).unwrap();
    assert_eq!(parsed.count, parsed.runs.len());
    parsed.runs.into_iter().map(|row| row.run_id).collect()
}

#[tokio::test]
async fn list_scopes_a_member_to_their_own_key_newest_first() {
    let store = seeded();
    let reply = call(
        Some(&store),
        Some(&member("hash-1")),
        Method::GET,
        "/v1/workflows/runs",
        "",
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(run_ids(&reply.body), ["run-2", "run-0"]);
    let recorded = store.recorded();
    let (filter, take) = recorded.filters.first().unwrap();
    assert_eq!(filter.created_by.as_deref(), Some("hash-1"));
    assert_eq!(*take, 50);
}

#[rstest]
#[case::master(master())]
#[case::proxy_admin(proxy_admin("hash-9"))]
#[case::view_only(viewer("hash-9"))]
#[tokio::test]
async fn list_reads_every_run_for_an_admin_view_caller(#[case] caller: Principal) {
    let store = seeded();
    let reply = call(
        Some(&store),
        Some(&caller),
        Method::GET,
        "/v1/workflows/runs",
        "",
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(run_ids(&reply.body), ["run-3", "run-2", "run-1", "run-0"]);
    assert_eq!(store.recorded().filters.first().unwrap().0.created_by, None);
}

#[tokio::test]
async fn list_for_a_member_without_a_key_is_empty_and_skips_the_store() {
    let store = seeded();
    let keyless = principal("virtual-key", vec![Role::TeamMember], None);
    let reply = call(
        Some(&store),
        Some(&keyless),
        Method::GET,
        "/v1/workflows/runs",
        "",
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert!(run_ids(&reply.body).is_empty());
    assert!(store.recorded().filters.is_empty());
}

#[rstest]
#[case::two("status=running,paused", Some(StatusFilter::In(vec!["running".to_owned(), "paused".to_owned()])))]
#[case::one("status=running", Some(StatusFilter::Equals("running".to_owned())))]
#[case::trimmed("status=%20running%20,%20paused%20", Some(StatusFilter::In(vec!["running".to_owned(), "paused".to_owned()])))]
#[case::trailing_comma("status=running,", Some(StatusFilter::In(vec!["running".to_owned(), String::new()])))]
#[case::empty("status=", None)]
#[case::absent("", None)]
#[tokio::test]
async fn list_turns_the_status_text_into_a_filter(
    #[case] query: &str,
    #[case] expected: Option<StatusFilter>,
) {
    let store = seeded();
    let uri = format!("/v1/workflows/runs?{query}");
    let reply = call(Some(&store), Some(&master()), Method::GET, &uri, "").await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(store.recorded().filters.first().unwrap().0.status, expected);
}

#[tokio::test]
async fn list_filters_by_status_in_the_store() {
    let store = seeded();
    let reply = call(
        Some(&store),
        Some(&master()),
        Method::GET,
        "/v1/workflows/runs?status=running,paused",
        "",
    )
    .await;
    assert_eq!(run_ids(&reply.body), ["run-1", "run-0"]);
}

#[rstest]
#[case::named("workflow_type=build", Some("build"), vec!["run-2"])]
#[case::empty_means_any("workflow_type=", None, vec!["run-3", "run-2", "run-1", "run-0"])]
#[tokio::test]
async fn list_filters_by_workflow_type(
    #[case] query: &str,
    #[case] expected: Option<&str>,
    #[case] ids: Vec<&str>,
) {
    let store = seeded();
    let uri = format!("/v1/workflows/runs?{query}");
    let reply = call(Some(&store), Some(&master()), Method::GET, &uri, "").await;
    assert_eq!(run_ids(&reply.body), ids);
    let recorded = store.recorded();
    assert_eq!(
        recorded.filters.first().unwrap().0.workflow_type.as_deref(),
        expected
    );
}

#[rstest]
#[case::default("", 50)]
#[case::lowest("limit=1", 1)]
#[case::highest("limit=250", 250)]
#[tokio::test]
async fn list_passes_a_limit_inside_the_bounds(#[case] query: &str, #[case] take: u8) {
    let store = seeded();
    let uri = format!("/v1/workflows/runs?{query}");
    let reply = call(Some(&store), Some(&master()), Method::GET, &uri, "").await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(store.recorded().filters.first().unwrap().1, take);
}

#[tokio::test]
async fn list_returns_at_most_limit_runs() {
    let store = seeded();
    let reply = call(
        Some(&store),
        Some(&master()),
        Method::GET,
        "/v1/workflows/runs?limit=1",
        "",
    )
    .await;
    assert_eq!(run_ids(&reply.body), ["run-3"]);
}

#[rstest]
#[case::zero("limit=0", "greater_than_equal")]
#[case::negative("limit=-5", "greater_than_equal")]
#[case::just_above("limit=251", "less_than_equal")]
#[case::beyond_a_byte("limit=300", "less_than_equal")]
#[case::huge("limit=99999999999", "less_than_equal")]
#[case::text("limit=abc", "int_parsing")]
#[case::empty("limit=", "int_parsing")]
#[tokio::test]
async fn list_rejects_a_limit_outside_the_bounds(#[case] query: &str, #[case] issue_type: &str) {
    let store = seeded();
    let uri = format!("/v1/workflows/runs?{query}");
    let reply = call(Some(&store), Some(&master()), Method::GET, &uri, "").await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
    let parsed: ValidationReply = serde_json::from_str(&reply.body).unwrap();
    let issue = parsed.detail.first().unwrap();
    assert_eq!(issue.issue_type, issue_type);
    assert_eq!(
        issue.loc,
        [
            LocReply::Name("query".to_owned()),
            LocReply::Name("limit".to_owned())
        ]
    );
    assert!(store.recorded().filters.is_empty());
}

#[tokio::test]
async fn list_validates_the_limit_before_it_looks_for_a_database() {
    let reply = call(
        None,
        Some(&master()),
        Method::GET,
        "/v1/workflows/runs?limit=0",
        "",
    )
    .await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn list_without_a_database_is_a_500_with_the_db_error() {
    let reply = call(None, Some(&master()), Method::GET, "/v1/workflows/runs", "").await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    let detail: DetailReply = serde_json::from_str(&reply.body).unwrap();
    assert_eq!(
        detail.detail,
        CommonProxyErrors::DbNotConnectedError.as_str()
    );
}

#[tokio::test]
async fn list_store_failure_is_a_500() {
    let store = Arc::new(FakeStore {
        state: Mutex::default(),
        failing: true,
        vanish_on_update: false,
    });
    let reply = call(
        Some(&store),
        Some(&master()),
        Method::GET,
        "/v1/workflows/runs",
        "",
    )
    .await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
}

#[tokio::test]
async fn list_without_a_caller_is_unauthenticated() {
    let store = seeded();
    let reply = call(Some(&store), None, Method::GET, "/v1/workflows/runs", "").await;
    assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
    assert!(store.recorded().filters.is_empty());
}
