use std::collections::BTreeMap;

use super::fixture::{environment_allowing, layout};
use crate::oidc::credential_dirs::resolve_oidc_file_path;
use crate::oidc::error::OidcError;

#[rstest::rstest]
#[case::empty("")]
#[case::bare_name("token")]
#[case::dot_relative("./token")]
#[case::parent_relative("../token")]
fn a_relative_path_is_refused_with_the_required_format(#[case] requested: &str) {
    let layout = layout();
    let error = resolve_oidc_file_path(requested, &layout.environment).unwrap_err();
    assert!(matches!(error, OidcError::RelativePath));
    assert!(
        error
            .to_string()
            .contains("oidc/file//var/run/secrets/<name>")
    );
}

#[test]
fn a_file_inside_the_allowed_directory_resolves_to_its_real_path() {
    let layout = layout();
    let requested = layout.allowed.join("token");
    let resolved = resolve_oidc_file_path(requested.to_str().unwrap(), &layout.environment);
    assert_eq!(resolved.unwrap(), requested);
}

#[test]
fn the_allowed_directory_itself_is_accepted() {
    let layout = layout();
    let resolved = resolve_oidc_file_path(layout.allowed.to_str().unwrap(), &layout.environment);
    assert_eq!(resolved.unwrap(), layout.allowed);
}

#[test]
fn a_file_that_does_not_exist_yet_is_accepted_inside_the_directory() {
    let layout = layout();
    let requested = layout.allowed.join("missing/token");
    let resolved = resolve_oidc_file_path(requested.to_str().unwrap(), &layout.environment);
    assert_eq!(resolved.unwrap(), requested);
}

#[test]
fn dot_segments_are_normalized_before_the_check() {
    let layout = layout();
    let requested = format!("{}/nested/./deeper/../token", layout.allowed.display());
    let resolved = resolve_oidc_file_path(&requested, &layout.environment).unwrap();
    assert_eq!(resolved, layout.allowed.join("nested/token"));
}

#[rstest::rstest]
#[case::sibling_directory("outside/secret")]
#[case::parent_of_allowed("")]
#[case::traversal_out("allowed/../outside/secret")]
#[case::traversal_from_inside("allowed/nested/../../outside/secret")]
#[case::name_prefix_of_allowed("allowed-evil/token")]
fn a_path_outside_the_allowed_directory_is_refused(#[case] relative_to_base: &str) {
    let layout = layout();
    let base = layout.allowed.parent().unwrap();
    let requested = format!("{}/{relative_to_base}", base.display());
    let error = resolve_oidc_file_path(&requested, &layout.environment).unwrap_err();
    assert!(matches!(error, OidcError::OutsideAllowedDirs));
    assert!(
        error
            .to_string()
            .contains("LITELLM_OIDC_ALLOWED_CREDENTIAL_DIRS")
    );
}

#[test]
fn any_one_of_several_allowed_directories_is_enough() {
    let layout = layout();
    let both = format!("{},{}", layout.allowed.display(), layout.outside.display());
    let requested = layout.outside.join("secret");
    let resolved =
        resolve_oidc_file_path(requested.to_str().unwrap(), &environment_allowing(&both));
    assert_eq!(resolved.unwrap(), requested);
}

#[test]
fn an_empty_allowlist_refuses_everything() {
    let layout = layout();
    let requested = layout.allowed.join("token");
    let error = resolve_oidc_file_path(requested.to_str().unwrap(), &environment_allowing(","))
        .unwrap_err();
    assert!(matches!(error, OidcError::OutsideAllowedDirs));
}

#[test]
fn the_root_directory_allows_every_absolute_path() {
    let layout = layout();
    let requested = layout.outside.join("secret");
    let resolved = resolve_oidc_file_path(requested.to_str().unwrap(), &environment_allowing("/"));
    assert_eq!(resolved.unwrap(), requested);
}

#[test]
fn the_default_mounts_apply_when_the_variable_is_absent() {
    let layout = layout();
    let requested = layout.allowed.join("token");
    let error = resolve_oidc_file_path(requested.to_str().unwrap(), &BTreeMap::new()).unwrap_err();
    assert!(matches!(error, OidcError::OutsideAllowedDirs));
    let default_file = resolve_oidc_file_path("/run/secrets/kubernetes.io/token", &BTreeMap::new());
    default_file.unwrap();
}

#[test]
fn a_path_over_the_limit_is_refused_before_touching_the_file_system() {
    let layout = layout();
    let requested = format!("{}/{}", layout.allowed.display(), "a/".repeat(2100));
    let error = resolve_oidc_file_path(&requested, &layout.environment).unwrap_err();
    assert!(matches!(error, OidcError::PathTooLong { limit: 4096 }));
}
