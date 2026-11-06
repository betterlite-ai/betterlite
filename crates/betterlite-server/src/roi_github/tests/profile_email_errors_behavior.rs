#![cfg(test)]

use rstest::rstest;

use super::support::{Reply, Script, github, json_reply, reply};

#[rstest]
#[case::not_found(Some(reply(404)))]
#[case::server_error(Some(reply(500)))]
#[case::unreachable(None)]
#[case::invalid_json(Some(json_reply(200, "<html>")))]
#[tokio::test]
async fn a_failed_profile_lookup_returns_the_fallback(#[case] answer: Option<Reply>) {
    let script = Script::new(answer.into_iter().collect());
    let github = github(script.clone());
    let email = github
        .profile_email("octocat", "fallback@example.com")
        .await;
    assert_eq!(email, "fallback@example.com");
    assert_eq!(script.seen().len(), 1);
}
