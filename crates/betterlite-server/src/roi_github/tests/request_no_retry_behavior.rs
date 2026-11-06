#![cfg(test)]

use std::time::Duration;

use tokio::time::Instant;

use axum::http::Method;
use rstest::rstest;

use super::support::{Script, github, json_reply, reply};

#[rstest]
#[case::post_rate_limit(
    Method::POST,
    429,
    "GitHub rate limit reached. Wait before syncing again. (HTTP 429)"
)]
#[case::get_bad_request(Method::GET, 400, "GitHub returned an error. (HTTP 400)")]
#[case::get_unauthorized(
    Method::GET,
    401,
    "Authentication failed. Check the configured GitHub token. (HTTP 401)"
)]
#[case::get_server_error(Method::GET, 500, "GitHub returned an error. (HTTP 500)")]
#[tokio::test(start_paused = true)]
async fn only_a_get_on_a_transient_status_is_retried(
    #[case] method: Method,
    #[case] status: u16,
    #[case] message: &str,
) {
    let script = Script::new(vec![reply(status), json_reply(200, "[]")]);
    let github = github(script.clone());
    let started = Instant::now();
    let error = github
        .request(method, "user/repos", &[], None)
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), message);
    assert_eq!(script.seen().len(), 1);
    assert_eq!(started.elapsed(), Duration::ZERO);
}

#[tokio::test]
async fn an_unreachable_transport_reports_the_connection_message() {
    let script = Script::new(Vec::new());
    let github = github(script.clone());
    let error = github
        .request(Method::GET, "user/repos", &[], None)
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Could not reach GitHub. Check the API URL and network connection."
    );
    assert_eq!(script.seen().len(), 1);
}
