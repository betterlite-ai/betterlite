#![cfg(test)]

use rstest::rstest;

use crate::roi_cache::key::settings_fingerprint;

use super::support::settings;

#[rstest]
#[case::one_slash("https://api.github.com/")]
#[case::many_slashes("https://api.github.com////")]
fn trailing_slashes_do_not_change_the_fingerprint(#[case] url: &str) {
    let mut slashed = settings();
    slashed.github_api_url = url.to_owned();
    assert_eq!(
        settings_fingerprint(&slashed).unwrap(),
        settings_fingerprint(&settings()).unwrap()
    );
}

#[test]
fn only_the_trailing_slashes_are_stripped() {
    let mut inner = settings();
    inner.github_api_url = "https://api.github.com/api/v3/".to_owned();
    let mut plain = settings();
    plain.github_api_url = "https://api.github.com/api/v3".to_owned();
    assert_eq!(
        settings_fingerprint(&inner).unwrap(),
        settings_fingerprint(&plain).unwrap()
    );
    assert_ne!(
        settings_fingerprint(&inner).unwrap(),
        settings_fingerprint(&settings()).unwrap()
    );
}
