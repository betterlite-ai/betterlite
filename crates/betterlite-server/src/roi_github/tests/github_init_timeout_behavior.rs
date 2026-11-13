#![cfg(test)]

use std::time::Duration;

use axum::http::Method;

use super::client::GITHUB_TIMEOUT;
use super::support::{Script, github, json_reply};

#[tokio::test]
async fn the_client_sets_a_forty_five_second_timeout() {
    let script = Script::new(vec![json_reply(200, "[]")]);
    let github = github(script);
    assert_eq!(github.timeout, Duration::from_secs(45));
    assert_eq!(GITHUB_TIMEOUT, Duration::from_secs(45));
}

#[tokio::test]
async fn every_request_carries_the_forty_five_second_timeout() {
    let script = Script::new(vec![json_reply(200, "[]")]);
    let github = github(script.clone());
    github
        .request(Method::GET, "user/repos", &[], None)
        .await
        .unwrap();
    assert_eq!(script.seen()[0].deadline, Duration::from_secs(45));
}
