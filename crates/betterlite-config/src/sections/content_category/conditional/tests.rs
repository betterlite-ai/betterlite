#![cfg(test)]

use std::fs;

use rstest::rstest;

use crate::sections::content_filter_records::{
    ContentFilterAction, ContentFilterCategoryConfig, ContentFilterSeverity,
};

#[test]
fn conditional_inheritance_obeys_severity_threshold() {
    super::tests::conditional_inheritance_is_severity_filtered();
}

#[rstest]
#[case::none("keywords: []\n", false)]
#[case::identifiers("identifier_words: [Child]\n", false)]
#[case::inheritance("inherit_from: base.yaml\n", false)]
#[case::additional("additional_block_words: [Extra]\n", false)]
#[case::inherited("identifier_words: [Child]\ninherit_from: base.yaml\n", true)]
#[case::extra("identifier_words: [Child]\nadditional_block_words: [Extra]\n", true)]
#[case::both(
    "identifier_words: [Child]\ninherit_from: base.yaml\nadditional_block_words: [Extra]\n",
    true
)]
fn conditional_loading_requires_identifiers_and_block_words(
    #[case] source: &str,
    #[case] expected: bool,
) {
    let temp = tempfile::tempdir().unwrap();
    fs::create_dir_all(temp.path().join("categories")).unwrap();
    fs::write(
        temp.path().join("categories/base.yaml"),
        "keywords: [{keyword: Danger, severity: high}]\n",
    )
    .unwrap();
    let path = temp.path().join("child.yaml");
    fs::write(&path, source).unwrap();
    let config = ContentFilterCategoryConfig {
        category: "child".to_owned(),
        enabled: true,
        action: ContentFilterAction::Mask,
        severity_threshold: ContentFilterSeverity::Medium,
        category_file: Some(path.display().to_string()),
    };
    let loaded = super::loading::load_categories(
        &[config],
        &[temp.path().to_path_buf()],
        temp.path(),
        false,
    );
    assert!(loaded.loaded_categories.contains_key("child"));
    assert_eq!(
        loaded.conditional_categories.contains_key("child"),
        expected
    );
    if let Some(conditional) = loaded.conditional_categories.get("child") {
        assert_eq!(conditional.identifier_words, ["child"]);
        assert_eq!(conditional.action, ContentFilterAction::Mask);
        assert_eq!(conditional.severity, "high");
    }
}
