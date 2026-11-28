#![cfg(test)]

use axum::http::StatusCode;
use rstest::rstest;

use crate::lens_access::tests::support::request;
use crate::lens_worker_routes::tests::support::{Harness, authorized, worker_call};

const UNAUTHENTICATED: &str = "Not authenticated";

#[rstest]
#[case::claim("POST", "/lens/worker/claim")]
#[case::progress("POST", "/lens/worker/l1/j1/progress")]
#[case::heartbeat("POST", "/lens/worker/l1/j1/heartbeat")]
#[case::sample("GET", "/lens/worker/l1/j1/sample")]
#[case::content("GET", "/lens/worker/l1/j1/content?execution_id=e1")]
#[case::model("POST", "/lens/worker/l1/j1/model")]
#[case::result("POST", "/lens/worker/l1/j1/result")]
#[tokio::test]
async fn every_worker_route_refuses_a_missing_bearer(#[case] method: &str, #[case] uri: &str) {
    let harness = Harness::new();
    let reply = worker_call(harness.app(), request(method, uri, None, None)).await;
    assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
    assert_eq!(reply.detail(), UNAUTHENTICATED);
    assert_eq!(reply.header("www-authenticate").as_deref(), Some("Bearer"));
}

#[tokio::test]
async fn the_router_mounts_no_admin_route() {
    let harness = Harness::new();
    let reply = worker_call(harness.app(), authorized("GET", "/lens", None)).await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
    let worker_root = worker_call(harness.app(), authorized("GET", "/lens/worker", None)).await;
    assert_eq!(worker_root.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn the_worker_routes_keep_their_methods() {
    let harness = Harness::new();
    let reply = worker_call(harness.app(), authorized("PUT", "/lens/worker/claim", None)).await;
    assert_eq!(reply.status, StatusCode::METHOD_NOT_ALLOWED);
    let wrong_method = worker_call(
        harness.app(),
        authorized("GET", "/lens/worker/claim?protocol_version=2", None),
    )
    .await;
    assert_eq!(wrong_method.status(), StatusCode::METHOD_NOT_ALLOWED);
    let wrong_path = worker_call(
        harness.app(),
        authorized("POST", "/lens/worker/l1/j1/sample", None),
    )
    .await;
    assert_eq!(wrong_path.status(), StatusCode::METHOD_NOT_ALLOWED);
}
