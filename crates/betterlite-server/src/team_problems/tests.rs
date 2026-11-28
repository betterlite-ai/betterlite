#![cfg(test)]

use std::str;

use axum::body::to_bytes;
use axum::http::header::CONTENT_TYPE;
use axum::response::IntoResponse as _;
use rstest::rstest;

use crate::list_api::problem::ManagementProblem;

use super::{forbidden, team_not_found};

#[test]
fn team_not_found_fields() {
    let problem = team_not_found("team-1");
    let detail = &problem.0;
    assert_eq!(detail.r#type, "urn:litellm:error:team-not-found");
    assert_eq!(detail.title, "Team not found");
    assert_eq!(detail.status, 404);
    assert_eq!(detail.detail, "Team id=team-1 does not exist in db");
    assert_eq!(detail.allowed, None);
}

#[test]
fn team_not_found_keeps_empty_id() {
    let problem = team_not_found("");
    assert_eq!(problem.0.detail, "Team id= does not exist in db");
}

#[rstest]
#[case::bulk_delete(
    "Call not allowed. User not proxy admin OR team admin OR org admin for this team. route='/management/v1/teams/team-1/members/bulk_delete'"
)]
#[case::bulk_update(
    "Call not allowed. User not proxy admin OR team admin OR org admin for this team. route='/management/v1/teams/team-1/members/bulk_update'"
)]
#[case::user_delete("Only PROXY_ADMIN or ORG_ADMIN users may delete users.")]
#[case::empty("")]
fn forbidden_keeps_detail(#[case] detail: &str) {
    let problem = forbidden(detail.to_owned());
    assert_eq!(problem.0.r#type, "urn:litellm:error:forbidden");
    assert_eq!(problem.0.title, "Forbidden");
    assert_eq!(problem.0.status, 403);
    assert_eq!(problem.0.detail, detail);
    assert_eq!(problem.0.allowed, None);
}

#[rstest]
#[case::team_not_found(team_not_found("team-1"))]
#[case::forbidden(forbidden("Only PROXY_ADMIN or ORG_ADMIN users may delete users.".to_owned()))]
fn display_is_detail(#[case] problem: ManagementProblem) {
    assert_eq!(problem.to_string(), problem.0.detail);
}

#[rstest]
#[case::team_not_found(
    team_not_found("team-1"),
    404,
    r#"{"type":"urn:litellm:error:team-not-found","title":"Team not found","status":404,"detail":"Team id=team-1 does not exist in db"}"#
)]
#[case::forbidden(
    forbidden("nope".to_owned()),
    403,
    r#"{"type":"urn:litellm:error:forbidden","title":"Forbidden","status":403,"detail":"nope"}"#
)]
#[tokio::test]
async fn problem_response_shape(
    #[case] problem: ManagementProblem,
    #[case] status: u16,
    #[case] expected: &str,
) {
    let response = problem.into_response();
    assert_eq!(response.status().as_u16(), status);
    assert_eq!(
        response.headers().get(CONTENT_TYPE).unwrap(),
        "application/problem+json"
    );
    let body = to_bytes(response.into_body(), 1024).await.unwrap();
    assert_eq!(str::from_utf8(&body).unwrap(), expected);
}
