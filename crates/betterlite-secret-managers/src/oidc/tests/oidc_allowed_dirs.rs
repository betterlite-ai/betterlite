use std::collections::BTreeMap;
use std::os::unix::fs::symlink;
use std::path::PathBuf;

use soft_canonicalize::soft_canonicalize;

use super::fixture::{environment_allowing, layout};
use crate::oidc::credential_dirs::oidc_allowed_dirs;
use crate::oidc::error::OidcError;

fn paths(texts: &[&str]) -> Vec<PathBuf> {
    texts.iter().map(PathBuf::from).collect()
}

#[test]
fn defaults_to_the_container_secret_mounts_when_the_variable_is_absent() {
    let expected = vec![
        soft_canonicalize("/var/run/secrets").unwrap(),
        soft_canonicalize("/run/secrets").unwrap(),
    ];
    assert_eq!(oidc_allowed_dirs(&BTreeMap::new()).unwrap(), expected);
}

#[test]
fn an_empty_variable_counts_as_unset() {
    let expected = oidc_allowed_dirs(&BTreeMap::new()).unwrap();
    assert_eq!(
        oidc_allowed_dirs(&environment_allowing("")).unwrap(),
        expected
    );
}

#[rstest::rstest]
#[case::one("/srv/a", &["/srv/a"])]
#[case::two_with_spaces(" /srv/a , /srv/b ", &["/srv/a", "/srv/b"])]
#[case::empty_entries_dropped("/srv/a,, ,/srv/b,", &["/srv/a", "/srv/b"])]
#[case::trailing_slash_and_dots("/srv/a/, /srv/b/./c/../d", &["/srv/a", "/srv/b/d"])]
#[case::repeated_slashes("/srv//a", &["/srv/a"])]
#[case::duplicates_kept("/srv/a,/srv/a", &["/srv/a", "/srv/a"])]
#[case::only_separators_means_no_directory(" , ,", &[])]
fn override_is_split_stripped_and_normalized(#[case] value: &str, #[case] expected: &[&str]) {
    let resolved = oidc_allowed_dirs(&environment_allowing(value)).unwrap();
    assert_eq!(resolved, paths(expected));
}

#[test]
fn a_symlinked_directory_is_replaced_by_its_target() {
    let layout = layout();
    let link = layout.outside.join("link");
    symlink(&layout.allowed, &link).unwrap();
    let resolved = oidc_allowed_dirs(&environment_allowing(link.to_str().unwrap())).unwrap();
    assert_eq!(resolved, vec![layout.allowed]);
}

#[test]
fn a_relative_directory_becomes_absolute() {
    let resolved = oidc_allowed_dirs(&environment_allowing("creds/here")).unwrap();
    let only = resolved.first().unwrap();
    assert!(only.is_absolute());
    assert!(only.ends_with("creds/here"));
}

#[test]
fn an_entry_over_the_path_limit_is_refused() {
    let long = format!("/{}", "a/".repeat(2100));
    let error = oidc_allowed_dirs(&environment_allowing(&long)).unwrap_err();
    assert!(matches!(error, OidcError::PathTooLong { limit: 4096 }));
}
