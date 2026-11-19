#![cfg(test)]

use axum::http::StatusCode;
use betterlite_roi::records::ROIRepositoriesResponse;

use crate::roi_connections::routes::repositories::{REPOSITORIES_PATH, TOO_LONG};
use crate::roi_connections::tests::support::{Answer, admin, call, get, harness};

fn path(raw: &str) -> String {
    format!("{REPOSITORIES_PATH}?{raw}")
}

#[tokio::test]
async fn query_parameter_filters_the_repositories() {
    let harness = harness().answers(vec![Answer::ok(
        r#"[{"full_name":"acme/widgets"},{"full_name":"acme/gadgets"}]"#,
    )]);
    let reply = call(harness.app(), get(&path("query=widget"), Some(&admin()))).await;
    assert_eq!(reply.status, StatusCode::OK);
    let response: ROIRepositoriesResponse = reply.json();
    assert_eq!(response.repositories.len(), 1);
    assert_eq!(response.repositories[0].name, "acme/widgets");
}

#[tokio::test]
async fn query_parameter_matches_case_insensitively() {
    let harness = harness().answers(vec![Answer::ok(r#"[{"full_name":"Acme/Widgets"}]"#)]);
    let reply = call(harness.app(), get(&path("query=WIDGET"), Some(&admin()))).await;
    let response: ROIRepositoriesResponse = reply.json();
    assert_eq!(response.repositories.len(), 1);
}

#[tokio::test]
async fn query_pages_github_by_one_hundred() {
    let harness = harness().answers(vec![Answer::ok("[]")]);
    let reply = call(harness.app(), get(&path("query=widget"), Some(&admin()))).await;
    assert_eq!(reply.status, StatusCode::OK);
    let seen = harness.wire.seen();
    assert_eq!(seen[0].url, "https://api.github.com/user/repos");
    assert!(
        seen[0]
            .query
            .contains(&("per_page".to_owned(), "100".to_owned()))
    );
    assert!(seen[0].query.contains(&("page".to_owned(), "1".to_owned())));
}

#[tokio::test]
async fn query_at_the_character_limit_is_accepted() {
    let harness = harness().answers(vec![Answer::ok("[]")]);
    let raw = format!("query={}", "a".repeat(200));
    let reply = call(harness.app(), get(&path(&raw), Some(&admin()))).await;
    assert_eq!(reply.status, StatusCode::OK);
}

#[tokio::test]
async fn query_over_the_character_limit_is_rejected() {
    let harness = harness();
    let raw = format!("query={}", "a".repeat(201));
    let reply = call(harness.app(), get(&path(&raw), Some(&admin()))).await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
    let issues = reply.issues();
    assert_eq!(issues[0].kind, "string_too_long");
    assert_eq!(issues[0].loc, ["query", "query"]);
    assert_eq!(issues[0].msg, TOO_LONG);
}
