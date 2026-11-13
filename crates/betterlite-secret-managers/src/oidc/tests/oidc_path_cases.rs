use std::fs;
use std::os::unix::fs::symlink;

use super::fixture::layout;
use crate::oidc::credential_dirs::resolve_oidc_file_path;
use crate::oidc::error::OidcError;

#[test]
fn a_relative_link_that_stays_inside_is_followed() {
    let layout = layout();
    symlink("nested/deeper/token", layout.allowed.join("short")).unwrap();
    let requested = layout.allowed.join("short");
    let resolved = resolve_oidc_file_path(requested.to_str().unwrap(), &layout.environment);
    assert_eq!(
        resolved.unwrap(),
        layout.allowed.join("nested/deeper/token")
    );
}

#[test]
fn an_absolute_link_that_stays_inside_is_followed() {
    let layout = layout();
    symlink(layout.allowed.join("token"), layout.allowed.join("alias")).unwrap();
    let requested = layout.allowed.join("alias");
    let resolved = resolve_oidc_file_path(requested.to_str().unwrap(), &layout.environment);
    assert_eq!(resolved.unwrap(), layout.allowed.join("token"));
}

#[test]
fn an_absolute_link_to_a_file_outside_is_refused() {
    let layout = layout();
    symlink(layout.outside.join("secret"), layout.allowed.join("escape")).unwrap();
    let requested = layout.allowed.join("escape");
    let error =
        resolve_oidc_file_path(requested.to_str().unwrap(), &layout.environment).unwrap_err();
    assert!(matches!(error, OidcError::OutsideAllowedDirs));
}

#[test]
fn a_relative_link_climbing_out_is_refused() {
    let layout = layout();
    symlink("../outside/secret", layout.allowed.join("climb")).unwrap();
    let requested = layout.allowed.join("climb");
    let error =
        resolve_oidc_file_path(requested.to_str().unwrap(), &layout.environment).unwrap_err();
    assert!(matches!(error, OidcError::OutsideAllowedDirs));
}

#[test]
fn a_nested_link_climbing_out_is_refused() {
    let layout = layout();
    symlink(
        "../../../outside/secret",
        layout.allowed.join("nested/deeper/climb"),
    )
    .unwrap();
    let requested = layout.allowed.join("nested/deeper/climb");
    let error =
        resolve_oidc_file_path(requested.to_str().unwrap(), &layout.environment).unwrap_err();
    assert!(matches!(error, OidcError::OutsideAllowedDirs));
}

#[test]
fn a_linked_directory_pointing_outside_blocks_every_file_below_it() {
    let layout = layout();
    symlink(&layout.outside, layout.allowed.join("dir")).unwrap();
    let requested = layout.allowed.join("dir/secret");
    let error =
        resolve_oidc_file_path(requested.to_str().unwrap(), &layout.environment).unwrap_err();
    assert!(matches!(error, OidcError::OutsideAllowedDirs));
}

#[test]
fn dot_dot_after_a_link_is_applied_to_the_link_target() {
    let layout = layout();
    symlink(layout.outside.join("sub"), layout.allowed.join("hop")).unwrap();
    let requested = format!("{}/hop/../secret", layout.allowed.display());
    let error = resolve_oidc_file_path(&requested, &layout.environment).unwrap_err();
    assert!(matches!(error, OidcError::OutsideAllowedDirs));
}

#[test]
fn a_chain_of_links_is_followed_to_its_end() {
    let layout = layout();
    symlink("two", layout.allowed.join("one")).unwrap();
    symlink("three", layout.allowed.join("two")).unwrap();
    symlink("token", layout.allowed.join("three")).unwrap();
    let requested = layout.allowed.join("one");
    let resolved = resolve_oidc_file_path(requested.to_str().unwrap(), &layout.environment);
    assert_eq!(resolved.unwrap(), layout.allowed.join("token"));
}

#[test]
fn a_dangling_link_to_the_outside_is_refused() {
    let layout = layout();
    symlink(
        layout.outside.join("not-there"),
        layout.allowed.join("dangling"),
    )
    .unwrap();
    let requested = layout.allowed.join("dangling");
    let error =
        resolve_oidc_file_path(requested.to_str().unwrap(), &layout.environment).unwrap_err();
    assert!(matches!(error, OidcError::OutsideAllowedDirs));
}

#[test]
fn a_link_loop_ends_with_an_error_instead_of_hanging() {
    let layout = layout();
    symlink("b", layout.allowed.join("a")).unwrap();
    symlink("a", layout.allowed.join("b")).unwrap();
    let requested = layout.allowed.join("a");
    let outcome = resolve_oidc_file_path(requested.to_str().unwrap(), &layout.environment);
    outcome.unwrap_err();
}

#[test]
fn a_path_below_a_regular_file_resolves_like_realpath_does() {
    let layout = layout();
    let requested = layout.allowed.join("token/child");
    let resolved = resolve_oidc_file_path(requested.to_str().unwrap(), &layout.environment);
    assert_eq!(resolved.unwrap(), requested);
}

#[test]
fn reaching_the_allowed_directory_through_a_link_to_it_is_accepted() {
    let layout = layout();
    let alias = layout.outside.join("alias");
    symlink(&layout.allowed, &alias).unwrap();
    let requested = alias.join("nested/deeper/token");
    let resolved = resolve_oidc_file_path(requested.to_str().unwrap(), &layout.environment);
    assert_eq!(
        resolved.unwrap(),
        layout.allowed.join("nested/deeper/token")
    );
}

#[test]
fn the_check_follows_the_file_system_at_call_time() {
    let layout = layout();
    let requested = layout.allowed.join("flip");
    fs::write(&requested, "plain file").unwrap();
    resolve_oidc_file_path(requested.to_str().unwrap(), &layout.environment).unwrap();
    fs::remove_file(&requested).unwrap();
    symlink(layout.outside.join("secret"), &requested).unwrap();
    let error =
        resolve_oidc_file_path(requested.to_str().unwrap(), &layout.environment).unwrap_err();
    assert!(matches!(error, OidcError::OutsideAllowedDirs));
}
