#![cfg(test)]

use std::fs;
use std::os::unix::fs::symlink;
use std::path::PathBuf;

use rstest::rstest;
use soft_canonicalize::soft_canonicalize;
use tempfile::TempDir;

use super::{
    FilenameError, PathError, is_within, join_within, safe_filename, safe_join, try_safe_join,
};

fn workspace() -> (TempDir, PathBuf) {
    let workspace = TempDir::new().unwrap();
    let base = workspace.path().join("base");
    fs::create_dir_all(base.join("inside")).unwrap();
    fs::write(base.join("inside/file.txt"), b"hi").unwrap();
    let outside = workspace.path().join("outside");
    fs::create_dir_all(&outside).unwrap();
    symlink(&outside, base.join("escape")).unwrap();
    symlink("inside", base.join("loop")).unwrap();
    (workspace, base)
}

#[rstest]
#[case::plain_descendant(&["inside", "file.txt"], true)]
#[case::symlink_resolving_inside(&["loop", "file.txt"], true)]
#[case::symlink_resolving_outside(&["escape", "secret.txt"], false)]
#[case::parent_traversal(&["..", "outside", "secret.txt"], false)]
fn safe_join_confines_to_base_dir(#[case] parts: &[&str], #[case] expect_ok: bool) {
    let (_workspace, base) = workspace();
    assert_eq!(safe_join(&base, parts).is_ok(), expect_ok);
}

#[test]
fn safe_join_returns_the_realpath_resolved_destination() {
    let (_workspace, base) = workspace();
    let resolved = safe_join(&base, &["inside", "file.txt"]).unwrap();
    let expected = soft_canonicalize(base.join("inside/file.txt")).unwrap();
    assert_eq!(resolved, expected);
}

#[test]
fn safe_join_rejects_null_bytes_in_any_part() {
    let (_workspace, base) = workspace();
    let error = safe_join(&base, &["inside", "bad\0name.txt"]).unwrap_err();
    assert!(matches!(error, PathError::NullByte));
}

#[test]
fn safe_join_names_both_paths_when_it_escapes() {
    let (workspace, base) = workspace();
    let error = safe_join(&base, &["escape", "secret.txt"]).unwrap_err();
    let PathError::Escaped {
        resolved,
        base: reported_base,
    } = error
    else {
        panic!("expected an Escaped error");
    };
    assert!(resolved.starts_with(workspace.path().join("outside")));
    assert_eq!(reported_base, soft_canonicalize(&base).unwrap());
}

#[rstest]
#[case::confined(&["inside", "file.txt"], true)]
#[case::escapes(&["escape", "secret.txt"], false)]
fn try_safe_join_swaps_the_value_error_for_an_option(
    #[case] parts: &[&str],
    #[case] expect_some: bool,
) {
    let (_workspace, base) = workspace();
    assert_eq!(try_safe_join(&base, parts).is_some(), expect_some);
}

#[test]
fn is_within_follows_symlinks_on_both_sides() {
    let (_workspace, base) = workspace();
    assert!(is_within(&base.join("inside"), &base));
    assert!(is_within(&base.join("loop"), &base));
    assert!(is_within(&base, &base));
    assert!(!is_within(&base.join("escape"), &base));
}

#[test]
fn safe_join_keeps_the_unresolved_remainder_for_a_symlink_loop() {
    let workspace = TempDir::new().unwrap();
    let base = workspace.path().join("base");
    fs::create_dir_all(&base).unwrap();
    symlink("loop", base.join("loop")).unwrap();
    let resolved = safe_join(&base, &["loop"]).unwrap();
    let canonical_base = soft_canonicalize(&base).unwrap();
    assert_eq!(resolved, canonical_base.join("loop"));
    assert!(is_within(&base.join("loop"), &base));
    assert!(try_safe_join(&base, &["loop"]).is_some());
}

#[rstest]
#[case::nested(&["sub", "file.txt"], true)]
#[case::parent_traversal_escapes(&["..", "secret.txt"], false)]
#[case::absolute_part_replaces_the_base(&["/etc/passwd"], false)]
#[case::no_parts_equals_the_base_itself(&[], false)]
fn join_within_checks_components_lexically(#[case] parts: &[&str], #[case] expect_some: bool) {
    let (_workspace, base) = workspace();
    assert_eq!(join_within(&base, parts).is_some(), expect_some);
}

#[test]
fn join_within_returns_the_symlink_itself_without_resolving_it() {
    let (_workspace, base) = workspace();
    let joined = join_within(&base, &["escape", "secret.txt"]).unwrap();
    assert_eq!(joined, base.join("escape/secret.txt"));
}

#[test]
fn join_within_rejects_null_bytes() {
    let (_workspace, base) = workspace();
    assert_eq!(join_within(&base, &["bad\0name.txt"]), None);
}

#[rstest]
#[case::simple("report.pdf", "report.pdf")]
#[case::nested_unix_path("uploads/2024/report.pdf", "report.pdf")]
fn safe_filename_extracts_basename(#[case] input: &str, #[case] expected: &str) {
    assert_eq!(safe_filename(input).unwrap(), expected);
}

#[rstest]
#[case::backslash_only("uploads\\report.pdf", "report.pdf")]
#[case::mixed_separators("uploads/nested\\report.pdf", "report.pdf")]
fn safe_filename_normalizes_backslash_separators(#[case] input: &str, #[case] expected: &str) {
    assert_eq!(safe_filename(input).unwrap(), expected);
}

#[rstest]
#[case::null_byte("bad\0name.txt", true)]
#[case::empty("", false)]
#[case::current_dir(".", false)]
#[case::parent_dir("..", false)]
#[case::trailing_slash("uploads/", false)]
fn safe_filename_rejects_empty_and_unsafe_names(#[case] input: &str, #[case] is_null_byte: bool) {
    let error = safe_filename(input).unwrap_err();
    assert_eq!(matches!(error, FilenameError::NullByte), is_null_byte);
}
