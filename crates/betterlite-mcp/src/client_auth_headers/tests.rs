#![cfg(test)]

pub(crate) mod support;

use rstest::rstest;

use super::client_has_passthrough_authorization;
use super::client_has_per_server_auth_header;
use support::{headers, oauth2, server};

#[rstest]
#[case::missing(None, &[], false)]
#[case::empty(Some(&[][..]), &[], false)]
#[case::lowercase_authorization(
    Some(&[("github", "authorization", "Bearer test-lower")][..]),
    &[],
    true
)]
#[case::uppercase_authorization(
    Some(&[("github", "AUTHORIZATION", "Bearer test-upper")][..]),
    &[],
    true
)]
#[case::empty_authorization_value(Some(&[("github", "Authorization", "")][..]), &[], true)]
#[case::resolved_without_authorization(
    Some(&[("github", "X-Api-Key", "test-key")][..]),
    &[],
    false
)]
#[case::group_credential(
    Some(&[("team-a", "Authorization", "Bearer test-group")][..]),
    &["team-a"],
    true
)]
#[case::ambiguous_groups(
    Some(
        &[
            ("team-a", "Authorization", "Bearer test-a"),
            ("team-b", "Authorization", "Bearer test-b")
        ][..]
    ),
    &["team-a", "team-b"],
    false
)]
fn per_server_authorization(
    #[case] entries: Option<&[(&str, &str, &str)]>,
    #[case] access_groups: &[&str],
    #[case] expected: bool,
) {
    let built = entries.map(headers);
    let record = server(Some("github"), None, access_groups);
    assert_eq!(
        client_has_per_server_auth_header(&record, built.as_ref()),
        expected,
    );
}

#[rstest]
#[case::top_level_authorization(Some(&[("Authorization", "Bearer test-top")][..]), None, true)]
#[case::top_level_any_case(Some(&[("authorization", "Bearer test-top")][..]), None, true)]
#[case::empty_top_level_falls_through(
    Some(&[][..]),
    Some(&[("github", "Authorization", "Bearer test-server")][..]),
    true
)]
#[case::unrelated_top_level_falls_through(
    Some(&[("X-Api-Key", "test-key")][..]),
    Some(&[("github", "Authorization", "Bearer test-server")][..]),
    true
)]
#[case::empty_top_level_without_a_server_header(Some(&[][..]), None, false)]
#[case::both_missing(None, None, false)]
fn passthrough_authorization(
    #[case] oauth2_entries: Option<&[(&str, &str)]>,
    #[case] per_server_entries: Option<&[(&str, &str, &str)]>,
    #[case] expected: bool,
) {
    let top_level = oauth2_entries.map(oauth2);
    let per_server = per_server_entries.map(headers);
    let record = server(Some("github"), None, &[]);
    assert_eq!(
        client_has_passthrough_authorization(&record, top_level.as_ref(), per_server.as_ref()),
        expected,
    );
}
