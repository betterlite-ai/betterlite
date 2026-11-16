#![cfg(test)]

use rstest::rstest;

use crate::roi_cache::key::cache_key;
use crate::roi_github::models::{GitHubHead, GitHubPullListItem, GitHubUser};

use super::support::{pull_item, settings};

const LITELLM_KEY: &str = "a5bcb6522b65fc6cba1182eb5fcfd63ed21430ffbfe746f21b3a9b916196ea3d";
const LITELLM_EMPTY_BODY_KEY: &str =
    "3ab2b14d748105450b0a2c49cdb52b507d6c7ac3b01605dd66b1f08ec5e0bdfc";

#[test]
fn the_key_is_the_digest_litellm_computes_for_the_same_pull() {
    let mut settings = settings();
    settings.github_api_url = "https://api.github.com/".to_owned();
    let mut pull = pull_item(7, Some("Dev"), Some("abc123"));
    pull.title = "Fix: \u{fc}n\u{ef} \"q\"".to_owned();
    pull.body = Some("Body\nline".to_owned());
    let key = cache_key(&settings, "ctx", "Octo/Repo", &pull).unwrap();
    assert_eq!(key.as_deref(), Some(LITELLM_KEY));
}

#[test]
fn a_null_body_hashes_as_an_empty_one_and_the_login_is_casefolded() {
    let mut pull = pull_item(7, Some("Stra\u{df}e"), Some("abc123"));
    pull.title = "Plain".to_owned();
    pull.body = None;
    let key = cache_key(&settings(), "ctx", "Octo/Repo", &pull).unwrap();
    assert_eq!(key.as_deref(), Some(LITELLM_EMPTY_BODY_KEY));
}

fn change_number(pull: &mut GitHubPullListItem) {
    pull.number = 8;
}

fn change_head(pull: &mut GitHubPullListItem) {
    pull.head = Some(GitHubHead {
        sha: "def456".to_owned(),
    });
}

fn change_title(pull: &mut GitHubPullListItem) {
    pull.title = "Changed".to_owned();
}

fn change_body(pull: &mut GitHubPullListItem) {
    pull.body = Some("Changed".to_owned());
}

fn change_login(pull: &mut GitHubPullListItem) {
    pull.user = Some(GitHubUser {
        login: Some("other".to_owned()),
    });
}

#[rstest]
#[case::number(change_number)]
#[case::head(change_head)]
#[case::title(change_title)]
#[case::body(change_body)]
#[case::login(change_login)]
fn every_pull_field_changes_the_key(#[case] change: fn(&mut GitHubPullListItem)) {
    let base = pull_item(7, Some("dev"), Some("abc123"));
    let mut changed = base.clone();
    change(&mut changed);
    assert_ne!(
        cache_key(&settings(), "ctx", "o/r", &changed).unwrap(),
        cache_key(&settings(), "ctx", "o/r", &base).unwrap()
    );
}

#[rstest]
#[case::context("other", "o/r")]
#[case::repo("ctx", "o/other")]
fn the_context_and_repo_change_the_key(#[case] context: &str, #[case] repo: &str) {
    let pull = pull_item(7, Some("dev"), Some("abc123"));
    assert_ne!(
        cache_key(&settings(), context, repo, &pull).unwrap(),
        cache_key(&settings(), "ctx", "o/r", &pull).unwrap()
    );
}

#[test]
fn case_and_trailing_slashes_do_not_change_the_key() {
    let base = pull_item(7, Some("dev"), Some("abc123"));
    let expected = cache_key(&settings(), "ctx", "o/r", &base).unwrap();
    let shouting = pull_item(7, Some("DEV"), Some("abc123"));
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
