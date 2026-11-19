#![cfg(test)]

use axum::http::StatusCode;
use betterlite_roi::records::ROIRepositoriesResponse;
use rstest::rstest;

use crate::roi_connections::routes::repositories::{
    GREATER_EQUAL, INT_PARSING, LESS_EQUAL, REPOSITORIES_PATH,
};
use crate::roi_connections::tests::support::{Answer, admin, call, get, harness};

fn path(raw: &str) -> String {
    format!("{REPOSITORIES_PATH}?{raw}")
}

#[tokio::test]
async fn pagination_defaults_to_the_first_page() {
    let harness = harness().answers(vec![Answer::ok("[]")]);
    let reply = call(harness.app(), get(REPOSITORIES_PATH, Some(&admin()))).await;
    let response: ROIRepositoriesResponse = reply.json();
    assert_eq!(response.page, 1);
    assert!(!response.has_more);
    let seen = harness.wire.seen();
    assert!(seen[0].query.contains(&("page".to_owned(), "1".to_owned())));
}

#[tokio::test]
async fn pagination_passes_the_requested_page() {
    let harness = harness().answers(vec![Answer::ok("[]")]);
    let reply = call(harness.app(), get(&path("page=3"), Some(&admin()))).await;
    let response: ROIRepositoriesResponse = reply.json();
    assert_eq!(response.page, 3);
    let seen = harness.wire.seen();
    assert!(seen[0].query.contains(&("page".to_owned(), "3".to_owned())));
}

#[tokio::test]
async fn pagination_reports_a_next_link() {
    let harness = harness().answers(vec![Answer::ok("[]").with_header(
        "link",
        "<https://api.github.com/user/repos?page=2>; rel=\"next\"",
    )]);
    let reply = call(harness.app(), get(REPOSITORIES_PATH, Some(&admin()))).await;
    let response: ROIRepositoriesResponse = reply.json();
    assert!(response.has_more);
}

#[tokio::test]
async fn pagination_accepts_the_last_page() {
    let harness = harness().answers(vec![Answer::ok("[]")]);
    let reply = call(harness.app(), get(&path("page=1000"), Some(&admin()))).await;
    assert_eq!(reply.status, StatusCode::OK);
}

#[rstest]
#[case::before_the_first("page=0", "greater_than_equal", GREATER_EQUAL)]
#[case::past_the_last("page=1001", "less_than_equal", LESS_EQUAL)]
#[case::not_a_number("page=abc", "int_parsing", INT_PARSING)]
#[tokio::test]
async fn pagination_rejects_an_invalid_page(
    #[case] raw: &str,
    #[case] kind: &str,
    #[case] msg: &str,
) {
    let harness = harness();
    let reply = call(harness.app(), get(&path(raw), Some(&admin()))).await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
    let issues = reply.issues();
    assert_eq!(issues[0].kind, kind);
    assert_eq!(issues[0].loc, ["query", "page"]);
    assert_eq!(issues[0].msg, msg);
}
