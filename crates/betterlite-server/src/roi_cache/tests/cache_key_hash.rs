#![cfg(test)]

use rstest::rstest;

use crate::roi_cache::key::cache_key;

use super::support::{pull_item, settings};

const LITELLM_KEY: &str = "a5bcb6522b65fc6cba1182eb5fcfd63ed21430ffbfe746f21b3a9b916196ea3d";
const LITELLM_EMPTY_BODY_KEY: &str =
    "3ab2b14d748105450b0a2c49cdb52b507d6c7ac3b01605dd66b1f08ec5e0bdfc";

#[test]
fn the_key_is_the_digest_litellm_computes_for_the_same_pull() {
    let mut settings = settings();
    settings.github_api_url = "https://api.github.com/".to_owned();
    let mut pull = pull_item(7, Some("Dev"), Some("abc123"));
    pull.title = "Fix: ünï \"q\"".to_owned();
    pull.body = Some("Body\nline".to_owned());
    let key = cache_key(&settings, "ctx", "Octo/Repo", &pull).unwrap();
    assert_eq!(key.as_deref(), Some(LITELLM_KEY));
}

#[test]
fn a_null_body_hashes_as_an_empty_one_and_the_login_is_casefolded() {
    let mut pull = pull_item(7, Some("Straße"), Some("abc123"));
    pull.title = "Plain".to_owned();
    pull.body = None;
    let key = cache_key(&settings(), "ctx", "Octo/Repo", &pull).unwrap();
    assert_eq!(key.as_deref(), Some(LITELLM_EMPTY_BODY_KEY));
}

#[rstest]
#[case::context("other", "o/r", 7, "abc123", "Plain", "Body", "dev")]
#[case::repo("ctx", "o/other", 7, "abc123", "Plain", "Body", "dev")]
#[case::number("ctx", "o/r", 8, "abc123", "Plain", "Body", "dev")]
#[case::head("ctx", "o/r", 7, "def456", "Plain", "Body", "dev")]
#[case::title("ctx", "o/r", 7, "abc123", "Changed", "Body", "dev")]
#[case::body("ctx", "o/r", 7, "abc123", "Plain", "Changed", "dev")]
#[case::login("ctx", "o/r", 7, "abc123", "Plain", "Body", "other")]
fn every_identity_field_changes_the_key(
    #[case] context: &str,
    #[case] repo: &str,
    #[case] number: i64,
    #[case] sha: &str,
    #[case] title: &str,
    #[case] body: &str,
    #[case] login: &str,
) {
    let mut base = pull_item(7, Some("dev"), Some("abc123"));
    base.title = "Plain".to_owned();
    let expected = cache_key(&settings(), "ctx", "o/r", &base).unwrap();
    let mut changed = pull_item(number, Some(login), Some(sha));
    changed.title = title.to_owned();
    changed.body = Some(body.to_owned());
    let key = cache_key(&settings(), context, repo, &changed).unwrap();
    assert_ne!(key, expected);
}

#[test]
fn case_and_trailing_slashes_do_not_change_the_key() {
    let mut base = pull_item(7, Some("dev"), Some("abc123"));
    base.title = "Plain".to_owned();
    let expected = cache_key(&settings(), "ctx", "o/r", &base).unwrap();
    let mut shouting = pull_item(7, Some("DEV"), Some("abc123"));
    shouting.title = "Plain".to_owned();
    let mut slashed = settings();
    slashed.github_api_url = "https://api.github.com///".to_owned();
    assert_eq!(
        cache_key(&slashed, "ctx", "O/R", &shouting).unwrap(),
        expected
    );
}

#[test]
fn the_key_is_a_lowercase_sha256_hexdigest() {
    let pull = pull_item(7, Some("dev"), Some("abc123"));
    let key = cache_key(&settings(), "ctx", "o/r", &pull)
        .unwrap()
        .unwrap();
    assert_eq!(key.len(), 64);
    assert!(
        key.chars()
            .all(|character| matches!(character, '0'..='9' | 'a'..='f'))
    );
}
