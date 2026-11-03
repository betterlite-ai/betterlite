#![cfg(test)]

use std::{
    fs,
    path::{Path, PathBuf},
    slice,
};

use crate::sections::content_filter_records::{
    ContentFilterAction, ContentFilterSeverity, category_types::CategoryFileData,
};

use super::{
    files::load_category_file,
    loading::should_apply_severity,
    model::CategoryConfig,
    paths::{assert_within_data_roots, resolve_category_file_path},
};

fn fixture() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
}

#[test]
pub(crate) fn category_config_lowercases_words_and_skips_invalid_regex() {
    let data = CategoryFileData {
        category_name: Some("x".to_owned()),
        description: None,
        default_action: None,
        keywords: Vec::new(),
        exceptions: vec!["BAD".to_owned()],
        identifier_words: vec!["Child".to_owned()],
        always_block_keywords: Vec::new(),
        inherit_from: None,
        additional_block_words: vec!["Word".to_owned()],
        phrase_patterns: vec!["[".to_owned(), "foo".to_owned()],
    };
    let category = CategoryConfig::from_data(data);
    assert_eq!(category.exceptions, ["bad"]);
    assert_eq!(category.identifier_words, ["child"]);
    assert_eq!(category.additional_block_words, ["word"]);
    assert_eq!(category.phrase_patterns.len(), 1);
}

#[test]
pub(crate) fn category_config_default_action_is_block() {
    let category = CategoryConfig::from_data(CategoryFileData {
        category_name: None,
        description: None,
        default_action: None,
        keywords: Vec::new(),
        exceptions: Vec::new(),
        identifier_words: Vec::new(),
        always_block_keywords: Vec::new(),
        inherit_from: None,
        additional_block_words: Vec::new(),
        phrase_patterns: Vec::new(),
    });
    assert_eq!(category.default_action, ContentFilterAction::Block);
}

#[test]
pub(crate) fn load_categories_resolves_custom_category_files() {
    use crate::sections::content_filter_records::ContentFilterCategoryConfig;
    let temp = fixture();
    let path = temp.path().join("category.yaml");
    fs::write(
        &path,
        "category_name: sample\nkeywords:\n  - keyword: danger\n    severity: high\n",
    )
    .unwrap();
    let config = ContentFilterCategoryConfig {
        category: "sample".to_owned(),
        enabled: true,
        action: ContentFilterAction::Mask,
        severity_threshold: ContentFilterSeverity::default(),
        category_file: Some(path.display().to_string()),
    };
    let result = super::loading::load_categories(
        &[config],
        &[temp.path().to_path_buf()],
        temp.path(),
        false,
    );
    assert_eq!(
        result.category_keywords.get("danger").map(|entry| entry.2),
        Some(ContentFilterAction::Mask)
    );
}

#[test]
pub(crate) fn conditional_inheritance_is_severity_filtered() {
    let temp = fixture();
    fs::create_dir_all(temp.path().join("categories")).unwrap();
    let inherited = temp.path().join("categories/base.yaml");
    fs::write(&inherited, "keywords:\n  - keyword: low-word\n    severity: low\n  - keyword: high-word\n    severity: high\n").unwrap();
    let child_data = CategoryFileData {
        category_name: Some("child".to_owned()),
        description: None,
        default_action: None,
        keywords: Vec::new(),
        exceptions: Vec::new(),
        identifier_words: vec!["kid".to_owned()],
        always_block_keywords: Vec::new(),
        inherit_from: Some("base".to_owned()),
        additional_block_words: vec!["extra".to_owned()],
        phrase_patterns: Vec::new(),
    };
    let conditional = super::loading::load_conditional_category(
        &CategoryConfig::from_data(child_data),
        ContentFilterAction::Block,
        ContentFilterSeverity::Medium,
        &[temp.path().to_path_buf()],
    );
    assert_eq!(conditional.block_words, ["extra", "high-word"]);
    assert_eq!(conditional.severity, "high");
}

#[test]
pub(crate) fn file_yaml_defaults_action_and_json_deduplicates_and_maps_severity() {
    let temp = fixture();
    let yaml = temp.path().join("cat.yaml");
    fs::write(&yaml, "keywords: []\n").unwrap();
    assert_eq!(
        load_category_file(&yaml).unwrap().default_action,
        ContentFilterAction::Block
    );
    let json = temp.path().join("harm.json");
    fs::write(
        &json,
        r#"[{"match":" Foo |foo|bar","severity":3},{"match":"baz","severity":"bad"}]"#,
    )
    .unwrap();
    let category = load_category_file(&json).unwrap();
    assert_eq!(category.keywords.len(), 3);
    assert_eq!(
        category.keywords[0].get("severity").map(String::as_str),
        Some("high")
    );
    assert_eq!(
        category.keywords[2].get("severity").map(String::as_str),
        Some("medium")
    );
}

#[test]
pub(crate) fn severity_thresholds_follow_low_medium_high_order() {
    assert!(should_apply_severity("medium", "low"));
    assert!(!should_apply_severity("low", "medium"));
    assert!(should_apply_severity("high", "medium"));
}

#[test]
pub(crate) fn path_resolution_uses_root_and_rejects_escape() {
    let temp = fixture();
    let root = temp.path().join("data");
    fs::create_dir_all(root.join("categories")).unwrap();
    fs::write(root.join("categories/a.yaml"), "keywords: []").unwrap();
    fs::write(root.join("categories/external.yaml"), "keywords: []").unwrap();
    let found = resolve_category_file_path(
        Path::new("prefix/categories/a.yaml"),
        slice::from_ref(&root),
        &root,
        false,
    )
    .unwrap();
    assert_eq!(found, root.join("categories/a.yaml"));
    let external = resolve_category_file_path(
        Path::new("mounted/categories/external.yaml"),
        slice::from_ref(&root),
        &root,
        true,
    )
    .unwrap();
    assert_eq!(external, root.join("categories/external.yaml"));
    let error = resolve_category_file_path(
        Path::new("../../outside"),
        slice::from_ref(&root),
        &root,
        false,
    )
    .unwrap_err();
    assert!(matches!(
        error,
        super::paths::CategoryPathError::OutsideRoots { .. }
    ));
}

#[test]
pub(crate) fn path_assertion_requires_root_containment() {
    let root = PathBuf::from("/data/categories");
    assert_within_data_roots(Path::new("/data/categories/a.yaml"), slice::from_ref(&root)).unwrap();
    let error = assert_within_data_roots(Path::new("/data/other/a.yaml"), &[root]).unwrap_err();
    assert!(matches!(
        error,
        super::paths::CategoryPathError::OutsideRoots { .. }
    ));
}

#[test]
pub(crate) fn partial_updates_survive_later_load_failure() {
    use crate::sections::content_filter_records::ContentFilterCategoryConfig;
    let temp = fixture();
    let good = temp.path().join("good.yaml");
    fs::write(&good, "keywords:\n  - keyword: safe\n    severity: high\n").unwrap();
    let make = |name: &str, path: &str| ContentFilterCategoryConfig {
        category: name.to_owned(),
        enabled: true,
        action: ContentFilterAction::Block,
        severity_threshold: ContentFilterSeverity::default(),
        category_file: Some(path.to_owned()),
    };
    let result = super::loading::load_categories(
        &[
            make("good", &good.display().to_string()),
            make("bad", "missing.yaml"),
        ],
        &[temp.path().to_path_buf()],
        temp.path(),
        false,
    );
    assert!(result.category_keywords.contains_key("safe"));
}
