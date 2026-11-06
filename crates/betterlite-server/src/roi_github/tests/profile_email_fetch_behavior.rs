#![cfg(test)]

use rstest::rstest;

use super::profile_email;
use super::support::{Script, github, json_reply};

#[rstest]
#[case::a_public_email("{\"email\":\"User@Example.COM\"}", "user@example.com")]
#[case::an_invalid_email("{\"email\":\"not-an-email\"}", "")]
#[case::a_null_email("{\"email\":null}", "")]
#[case::a_missing_email("{}", "")]
#[case::a_github_noreply_address("{\"email\":\"12345+octo@users.noreply.github.com\"}", "")]
#[tokio::test]
async fn the_profile_email_is_normalized_before_it_is_returned(
    #[case] body: &str,
    #[case] expected: &str,
) {
    let script = Script::new(vec![json_reply(200, body)]);
    let github = github(script.clone());
    let email = profile_email::profile_email(&github, "octocat", "fallback@example.com").await;
    assert_eq!(email, expected);
    let seen = script.seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].url, "https://api.github.com/users/octocat");
}

#[tokio::test]
async fn the_login_is_percent_encoded_into_the_profile_path() {
    let script = Script::new(vec![json_reply(200, "{}")]);
    let github = github(script.clone());
    profile_email::profile_email(&github, "octo cat/../", "").await;
    assert_eq!(
        script.seen()[0].url,
        "https://api.github.com/users/octo%20cat%2F..%2F"
    );
}
