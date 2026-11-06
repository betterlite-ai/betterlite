#![cfg(test)]

use super::support::{Script, github, json_reply, reply};

#[tokio::test]
async fn a_profile_is_fetched_once_and_reused_for_any_letter_case() {
    let script = Script::new(vec![json_reply(200, "{\"email\":\"User@Example.COM\"}")]);
    let github = github(script.clone());
    let first = github
        .profile_email("OctoCat", "fallback@example.com")
        .await;
    let second = github.profile_email("octocat", "other@example.com").await;
    let third = github.profile_email("OCTOCAT", "").await;
    assert_eq!(first, "user@example.com");
    assert_eq!(second, "user@example.com");
    assert_eq!(third, "user@example.com");
    assert_eq!(script.seen().len(), 1);
}

#[tokio::test]
async fn a_failed_lookup_is_cached_and_the_current_fallback_is_returned() {
    let script = Script::new(vec![reply(404)]);
    let github = github(script.clone());
    let first = github.profile_email("octocat", "first@example.com").await;
    let second = github.profile_email("octocat", "second@example.com").await;
    assert_eq!(first, "first@example.com");
    assert_eq!(second, "second@example.com");
    assert_eq!(script.seen().len(), 1);
}

#[tokio::test]
async fn an_invalid_profile_email_is_cached_as_an_empty_address() {
    let script = Script::new(vec![json_reply(200, "{\"email\":\"broken\"}")]);
    let github = github(script.clone());
    let first = github
        .profile_email("octocat", "fallback@example.com")
        .await;
    let second = github
        .profile_email("octocat", "fallback@example.com")
        .await;
    assert_eq!(first, "");
    assert_eq!(second, "");
    assert_eq!(script.seen().len(), 1);
}
