#![cfg(test)]

use super::Op;
use crate::helpers::Helper as _;

#[test]
fn loads_all_patterns() {
    let result = Op::decode_and_run(&serde_json::json!({"function": "catalog"})).unwrap();
    assert_eq!(result["names"].as_array().unwrap().len(), 82);
}

use std::fs;
use std::os::unix::fs::symlink;
use std::path::PathBuf;

use rstest::rstest;
use tempfile::TempDir;

use betterlite_config::sections::content_filter_catalog::categories::{
    available_content_categories, category_dirs, category_entry, find_category_file,
};
use betterlite_config::sections::content_filter_records::ContentFilterAction;

fn root_with_categories(files: &[(&str, &str)]) -> (TempDir, PathBuf) {
    let tmp = TempDir::new().unwrap();
    let categories = tmp.path().join("categories");
    fs::create_dir_all(&categories).unwrap();
    for (name, content) in files {
        fs::write(categories.join(name), content).unwrap();
    }
    let root = tmp.path().to_path_buf();
    (tmp, root)
}

#[test]
fn category_dirs_lists_only_roots_that_have_a_categories_folder() {
    let (_bundled_tmp, bundled) = root_with_categories(&[]);
    let empty_tmp = TempDir::new().unwrap();
    let roots = [bundled.as_path(), empty_tmp.path()];
    let dirs = category_dirs(&roots);
    assert_eq!(dirs, vec![bundled.join("categories")]);
}

#[test]
fn find_category_file_prefers_yaml_over_json_in_the_same_root() {
    let (_tmp, root) =
        root_with_categories(&[("foo.yaml", "category_name: foo\n"), ("foo.json", "{}")]);
    let roots = [root.as_path()];
    let found = find_category_file("foo", &roots).unwrap();
    assert_eq!(found, root.join("categories/foo.yaml"));
}

#[test]
fn find_category_file_checks_the_bundled_root_before_the_legacy_root() {
    let (_bundled_tmp, bundled) = root_with_categories(&[("foo.json", "{}")]);
    let (_legacy_tmp, legacy) = root_with_categories(&[("foo.yaml", "category_name: foo\n")]);
    let roots = [bundled.as_path(), legacy.as_path()];
    let found = find_category_file("foo", &roots).unwrap();
    assert_eq!(found, bundled.join("categories/foo.json"));
}

#[rstest]
#[case::parent_traversal("../x")]
#[case::absolute_path("/abs/path")]
fn find_category_file_rejects_escaping_names(#[case] category_name: &str) {
    let (_tmp, root) = root_with_categories(&[("x.yaml", "category_name: x\n")]);
    let roots = [root.as_path()];
    assert_eq!(find_category_file(category_name, &roots), None);
}

#[test]
fn category_entry_skips_a_yaml_file_with_no_category_name() {
    let (_tmp, root) = root_with_categories(&[("broken.yaml", "display_name: Broken\n")]);
    let dir = root.join("categories");
    assert!(category_entry(&dir, "broken.yaml").is_none());
}

#[test]
fn category_entry_skips_a_yaml_file_that_fails_to_parse() {
    let (_tmp, root) = root_with_categories(&[("broken.yaml", "category_name: [unterminated\n")]);
    let dir = root.join("categories");
    assert!(category_entry(&dir, "broken.yaml").is_none());
}

#[test]
fn category_entry_builds_an_auto_display_name_for_a_plain_json_category() {
    let (_tmp, root) = root_with_categories(&[("harm_other.json", "{}")]);
    let dir = root.join("categories");
    let entry = category_entry(&dir, "harm_other.json").unwrap();
    assert_eq!(entry.name, "harm_other");
    assert_eq!(entry.display_name, "Harm Other");
    assert_eq!(entry.default_action, ContentFilterAction::Block);
}

#[test]
fn category_entry_hardcodes_the_harm_toxic_abuse_json_display_name() {
    let (_tmp, root) = root_with_categories(&[("harm_toxic_abuse.json", "{}")]);
    let dir = root.join("categories");
    let entry = category_entry(&dir, "harm_toxic_abuse.json").unwrap();
    assert_eq!(entry.display_name, "Harmful Toxic Abuse");
}

#[test]
fn category_files_load_by_name_through_a_symlink_and_return_the_link_itself() {
    let (_tmp, root) = root_with_categories(&[("real.yaml", "category_name: real\n")]);
    let dir = root.join("categories");
    symlink(dir.join("real.yaml"), dir.join("aliased.yaml")).unwrap();
    let roots = [root.as_path()];
    let found = find_category_file("aliased", &roots).unwrap();
    assert_eq!(found, dir.join("aliased.yaml"));
    let entry = category_entry(&dir, "aliased.yaml").unwrap();
    assert_eq!(entry.name, "real");
}

#[test]
fn available_content_categories_prefers_the_bundled_root_for_a_same_named_category() {
    // A same-named category in each real root proves precedence, not just uniqueness.
    let (_tmp_bundled, bundled) = root_with_categories(&[(
        "shared.yaml",
        "category_name: shared\ndisplay_name: Bundled Shared\n",
    )]);
    let (_tmp_legacy, legacy) = root_with_categories(&[(
        "shared.yaml",
        "category_name: shared\ndisplay_name: Legacy Shared\n",
    )]);
    let roots = [bundled.as_path(), legacy.as_path()];
    let entries = available_content_categories(&roots);
    let shared: Vec<_> = entries
        .iter()
        .filter(|entry| entry.name == "shared")
        .collect();
    assert_eq!(shared.len(), 1);
    assert_eq!(shared[0].display_name, "Bundled Shared");
}

#[test]
fn json_titles_words_after_punctuation_and_digits() {
    let entry = category_entry(std::path::Path::new("unused"), "mixed-UP_42title.json").unwrap();
    assert_eq!(entry.display_name, "Mixed-Up 42Title");
}
