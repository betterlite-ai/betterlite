#![cfg(test)]

use rstest::rstest;

use crate::roi_cache::key::cache_key;
use crate::roi_github::models::{GitHubHead, GitHubUser};

use super::support::{pull_item, settings};

#[rstest]
#[case::no_head(None, Some("dev"))]
#[case::empty_head_sha(Some(""), Some("dev"))]
#[case::no_user(Some("abc123"), None)]
#[case::empty_login(Some("abc123"), Some(""))]
fn cache_key_none_cases(#[case] sha: Option<&str>, #[case] login: Option<&str>) {
    let pull = pull_item(7, login, sha);
    assert_eq!(cache_key(&settings(), "ctx", "o/r", &pull).unwrap(), None);
}

#[test]
fn a_user_without_a_login_has_no_key() {
    let mut pull = pull_item(7, Some("dev"), Some("abc123"));
    pull.user = Some(GitHubUser { login: None });
    assert_eq!(cache_key(&settings(), "ctx", "o/r", &pull).unwrap(), None);
}

#[test]
fn complete_metadata_has_a_key() {
    let pull = pull_item(7, Some("dev"), Some("abc123"));
    assert!(
        cache_key(&settings(), "ctx", "o/r", &pull)
            .unwrap()
            .is_some()
    );
}

#[test]
fn a_head_without_a_sha_reads_as_an_empty_one() {
    let mut pull = pull_item(7, Some("dev"), Some("abc123"));
    pull.head = Some(GitHubHead::default());
    assert_eq!(cache_key(&settings(), "ctx", "o/r", &pull).unwrap(), None);
}
