#![cfg(test)]

use rstest::rstest;

use super::profile_email;
use super::support::{Reply, Script, github, json_reply, reply};

#[rstest]
#[case::not_found_without_a_body(Some(reply(404)))]
#[case::not_found_with_a_body(Some(json_reply(404, "{\"message\":\"Not Found\"}")))]
#[case::server_error_without_a_body(Some(reply(500)))]
#[case::server_error_holding_an_email(Some(json_reply(500, "{\"email\":\"dev@example.com\"}")))]
#[case::unreachable(None)]
#[case::invalid_json(Some(json_reply(200, "<html>")))]
#[tokio::test]
async fn a_failed_profile_lookup_returns_the_fallback(#[case] answer: Option<Reply>) {
    let script = Script::new(answer.into_iter().collect());
    let github = github(script.clone());
    let email = profile_email::profile_email(&github, "octocat", "fallback@example.com").await;
    assert_eq!(email, "fallback@example.com");
    assert_eq!(script.seen().len(), 1);
}

#[tokio::test]
async fn the_profile_lookup_is_sent_once_and_never_retried() {
    let script = Script::new(vec![
        reply(503),
        json_reply(200, "{\"email\":\"dev@example.com\"}"),
    ]);
    let github = github(script.clone());
    let email = profile_email::profile_email(&github, "octocat", "fallback@example.com").await;
    assert_eq!(email, "fallback@example.com");
    assert_eq!(script.seen().len(), 1);
}
