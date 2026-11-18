#![cfg(test)]

use axum::http::StatusCode;

use crate::roi_connections::routes::repositories::REPOSITORIES_PATH;
use crate::roi_connections::tests::support::{Answer, admin, call, get, harness};

#[tokio::test]
async fn source_error_http_status_is_a_502() {
    let harness = harness().answers(vec![Answer::status(401)]);
    let reply = call(harness.app(), get(REPOSITORIES_PATH, Some(&admin()))).await;
    assert_eq!(reply.status, StatusCode::BAD_GATEWAY);
    assert_eq!(
        reply.detail(),
        "Authentication failed. Check the configured GitHub token. (HTTP 401)"
    );
}

#[tokio::test]
async fn source_error_unexpected_page_is_a_502() {
    let harness = harness().answers(vec![Answer::ok("{}")]);
    let reply = call(harness.app(), get(REPOSITORIES_PATH, Some(&admin()))).await;
    assert_eq!(reply.status, StatusCode::BAD_GATEWAY);
    assert_eq!(
        reply.detail(),
        "GitHub returned an unexpected repository list."
    );
}

#[tokio::test]
async fn source_error_transport_is_a_502() {
    let harness = harness();
    let reply = call(harness.app(), get(REPOSITORIES_PATH, Some(&admin()))).await;
    assert_eq!(reply.status, StatusCode::BAD_GATEWAY);
    assert_eq!(
        reply.detail(),
        "Could not reach GitHub. Check the API URL and network connection."
    );
}
