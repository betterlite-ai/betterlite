#![cfg(test)]

use rstest::rstest;

use super::{posix_dirname, posix_normpath, resolve_include_object_key};

#[rstest]
#[case::nested("team/a.yaml", "../shared/models.yaml", "shared/models.yaml")]
#[case::root_relative_kept("a.yaml", "../shared/models.yaml", "../shared/models.yaml")]
#[case::same_prefix("team/a.yaml", "models.yaml", "team/models.yaml")]
#[case::same_prefix_nested("team/sub/a.yaml", "models.yaml", "team/sub/models.yaml")]
#[case::dot_file("team/a.yaml", "./models.yaml", "team/models.yaml")]
#[case::dot_directory("team/a.yaml", "sub/./models.yaml", "team/sub/models.yaml")]
#[case::parent_directory("team/a.yaml", "sub/../models.yaml", "team/models.yaml")]
#[case::parent_above_prefix("a/b/c.yaml", "../../d.yaml", "d.yaml")]
#[case::double_slash_in_declaring_key("team//a.yaml", "b.yaml", "team/b.yaml")]
#[case::trailing_slash_in_declaring_key("team/a/", "b.yaml", "team/a/b.yaml")]
#[case::empty_entry_from_root("a.yaml", "", ".")]
#[case::empty_entry_from_prefix("team/a.yaml", "", "team")]
fn resolves_relative_entries_against_the_declaring_key(
    #[case] config_object_key: &str,
    #[case] include_entry: &str,
    #[case] expected: &str,
) {
    assert_eq!(
        resolve_include_object_key(config_object_key, include_entry),
        expected
    );
}

#[rstest]
#[case::slash("team/a.yaml", "/shared/models.yaml", "shared/models.yaml")]
#[case::nested_absolute("team/a.yaml", "/team/shared.yaml", "team/shared.yaml")]
#[case::dotdot_absolute("team/a.yaml", "/a/../b.yaml", "b.yaml")]
#[case::up_above_root("team/a.yaml", "/../up.yaml", "up.yaml")]
#[case::doubled_slash("team/a.yaml", "//x.yaml", "x.yaml")]
#[case::tripled_slash("team/a.yaml", "///x.yaml", "x.yaml")]
#[case::dot_segments("team/a.yaml", "/./x.yaml", "x.yaml")]
#[case::root_only("team/a.yaml", "/", "")]
#[case::sibling_prefix("/team/a.yaml", "/other/b.yaml", "other/b.yaml")]
fn resolves_a_leading_slash_from_the_bucket_root(
    #[case] config_object_key: &str,
    #[case] include_entry: &str,
    #[case] expected: &str,
) {
    assert_eq!(
        resolve_include_object_key(config_object_key, include_entry),
        expected
    );
}

#[test]
fn a_root_level_key_has_no_directory() {
    assert_eq!(
        resolve_include_object_key("config.yaml", "models.yml"),
        "models.yml"
    );
}

#[test]
fn a_declaring_key_with_two_leading_slashes_keeps_them() {
    assert_eq!(resolve_include_object_key("//a.yaml", "b.yaml"), "//b.yaml");
}

#[test]
fn a_single_leading_slash_declaring_key_resolves_from_the_root() {
    assert_eq!(resolve_include_object_key("/a.yaml", "b.yaml"), "/b.yaml");
}

#[test]
fn a_declaring_key_with_three_leading_slashes_resolves_from_the_root() {
    assert_eq!(resolve_include_object_key("///a.yaml", "b.yaml"), "/b.yaml");
}

#[test]
fn dirname_keeps_the_prefix_up_to_the_last_slash() {
    assert_eq!(posix_dirname("/a/b"), "/a");
    assert_eq!(posix_dirname("a/b"), "a");
    assert_eq!(posix_dirname("a"), "");
    assert_eq!(posix_dirname(""), "");
    assert_eq!(posix_dirname("/"), "/");
    assert_eq!(posix_dirname("//"), "//");
}

#[test]
fn normpath_keeps_two_leading_slashes_and_collapses_three() {
    assert_eq!(posix_normpath("//a"), "//a");
    assert_eq!(posix_normpath("///a"), "/a");
    assert_eq!(posix_normpath(""), ".");
}
