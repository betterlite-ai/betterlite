#![cfg(test)]

use std::{fs, sync::Arc};

use rstest::rstest;
use tracing::subscriber::with_default;

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
        &super::Settings::default(),
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

#[rstest]
#[case::missing(None, true)]
#[case::invalid(Some("keywords: ["), false)]
#[case::malformed(Some("keywords: [{keyword: broken}]"), false)]
#[case::missing_word(Some("keywords: [{severity: high}]"), false)]
fn inheritance_failure_preserves_parent(#[case] source: Option<&str>, #[case] keep: bool) {
    let temp = tempfile::tempdir().unwrap();
    fs::create_dir_all(temp.path().join("categories")).unwrap();
    if let Some(text) = source {
        fs::write(temp.path().join("categories/base.yaml"), text).unwrap();
    }
    let path = temp.path().join("child.yaml");
    fs::write(&path, "identifier_words: [Child]\ninherit_from: base.yaml\nadditional_block_words: [Extra]\nkeywords: [{keyword: regular, severity: high}]\n").unwrap();
    let config: ContentFilterCategoryConfig = serde_saphyr::from_str(&format!(
        "category: child\naction: BLOCK\ncategory_file: {}\n",
        path.display()
    ))
    .unwrap();
    let log = tempfile::NamedTempFile::new().unwrap();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_writer(Arc::new(log.reopen().unwrap()))
        .finish();
    let loaded = with_default(subscriber, || {
        super::loading::load_categories(
            &[config],
            &[temp.path().to_path_buf()],
            temp.path(),
            &super::Settings::default(),
        )
    });
    assert!(loaded.loaded_categories.contains_key("child"));
    assert!(loaded.category_keywords.contains_key("regular"));
    assert_eq!(loaded.conditional_categories.contains_key("child"), keep);
    let output = fs::read_to_string(log.path()).unwrap();
    assert!(output.contains(if keep { "WARN" } else { "ERROR" }));
    assert!(output.contains("base"));
    if keep {
        assert!(output.contains(&temp.path().display().to_string()));
        assert_eq!(
            loaded.conditional_categories["child"].block_words,
            ["extra"]
        );
    }
}
