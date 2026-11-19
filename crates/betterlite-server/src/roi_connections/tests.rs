#![cfg(test)]

pub(crate) mod support;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use rstest::rstest;

use self::support::{Answer, admin, call, get, harness, post};
use crate::roi_connections::routes::connections::CONNECTIONS_TEST_PATH;
use crate::roi_connections::routes::repositories::REPOSITORIES_PATH;

#[tokio::test]
async fn oneshot_repositories_route_answers_the_router() {
    let harness = harness().answers(vec![Answer::ok("[{\"full_name\":\"acme/widgets\"}]")]);
    let reply = call(harness.app(), get(REPOSITORIES_PATH, Some(&admin()))).await;
    assert_eq!(reply.status, StatusCode::OK);
}

#[tokio::test]
async fn oneshot_connections_route_answers_the_router() {
    let harness = harness();
    let reply = call(harness.app(), post(CONNECTIONS_TEST_PATH, Some(&admin()))).await;
    assert_eq!(reply.status, StatusCode::CONFLICT);
}

#[tokio::test]
async fn oneshot_unknown_path_is_not_found() {
    let harness = harness();
    let reply = call(
        harness.app(),
        get("/roi-calculator/unknown", Some(&admin())),
    )
    .await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
}

#[rstest]
#[case::repositories_refuses_post(REPOSITORIES_PATH, "POST")]
#[case::connections_refuses_get(CONNECTIONS_TEST_PATH, "GET")]
#[tokio::test]
async fn oneshot_routes_refuse_the_other_method(#[case] path: &str, #[case] method: &str) {
    let harness = harness();
    let request = Request::builder()
        .method(method)
        .uri(path)
        .body(Body::empty())
        .expect("request");
    let reply = call(harness.app(), request).await;
    assert_eq!(reply.status, StatusCode::METHOD_NOT_ALLOWED);
}
