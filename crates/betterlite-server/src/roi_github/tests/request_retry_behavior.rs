#![cfg(test)]

use std::time::Duration;

use tokio::time::Instant;

use axum::http::Method;
use rstest::rstest;

use super::support::{Script, github, json_reply, reply};

const OK: &str = "[]";

#[rstest]
#[case::too_many_requests(429)]
#[case::bad_gateway(502)]
#[case::unavailable(503)]
#[case::gateway_timeout(504)]
#[tokio::test(start_paused = true)]
async fn a_get_retries_the_transient_status_twice(#[case] status: u16) {
    let script = Script::new(vec![reply(status), reply(status), json_reply(200, OK)]);
    let github = github(script.clone());
    let response = github
        .request(Method::GET, "user/repos", &[], None)
        .await
        .unwrap();
    assert_eq!(response.status, 200);
    assert_eq!(script.seen().len(), 3);
}

#[tokio::test(start_paused = true)]
async fn the_two_retries_wait_half_a_second_then_one_second() {
    let script = Script::new(vec![reply(429), reply(503), json_reply(200, OK)]);
    let github = github(script.clone());
    let started = Instant::now();
    let response = github
        .request(Method::GET, "user/repos", &[], None)
        .await
        .unwrap();
    assert_eq!(response.status, 200);
    assert_eq!(started.elapsed(), Duration::from_millis(1500));
    assert_eq!(script.seen().len(), 3);
}

#[tokio::test(start_paused = true)]
async fn a_third_failed_attempt_returns_the_status_error() {
    let script = Script::new(vec![reply(429), reply(429), reply(429), reply(429)]);
    let github = github(script.clone());
    let error = github
        .request(Method::GET, "user/repos", &[], None)
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "GitHub rate limit reached. Wait before syncing again. (HTTP 429)"
    );
    assert_eq!(script.seen().len(), 3);
}
