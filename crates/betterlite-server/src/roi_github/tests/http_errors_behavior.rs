#![cfg(test)]

use axum::http::Method;
use rstest::rstest;

use super::support::{Reply, Script, github, json_reply, reply};

#[rstest]
#[case::unauthorized(vec![reply(401)], "Authentication failed. Check the configured GitHub token. (HTTP 401)")]
#[case::forbidden(vec![reply(403)], "GitHub denied access or reached a rate limit. Check token permissions and organization approval. (HTTP 403)")]
#[case::not_found(vec![reply(404)], "GitHub repository or organization not found. Check its name, token access, and API URL. (HTTP 404)")]
#[case::rate_limited(vec![reply(429), reply(429), reply(429)], "GitHub rate limit reached. Wait before syncing again. (HTTP 429)")]
#[case::bad_request(vec![reply(400)], "GitHub returned an error. (HTTP 400)")]
#[case::server_error(vec![reply(500)], "GitHub returned an error. (HTTP 500)")]
#[tokio::test(start_paused = true)]
async fn http_failures_carry_litellm_s_labeled_message(
    #[case] replies: Vec<Reply>,
    #[case] expected: &str,
) {
    let script = Script::new(replies);
    let github = github(script.clone());
    let error = github
        .request(Method::GET, "user/repos", &[], None)
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), expected);
}

#[tokio::test]
async fn a_successful_answer_is_returned_unchanged() {
    let script = Script::new(vec![json_reply(201, "{\"ok\":true}")]);
    let github = github(script.clone());
    let response = github
        .request(Method::POST, "user/repos", &[], None)
        .await
        .unwrap();
    assert_eq!(response.status, 201);
    assert_eq!(response.body, b"{\"ok\":true}");
}
