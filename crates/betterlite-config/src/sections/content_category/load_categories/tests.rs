#![cfg(test)]

use std::{fs, slice, sync::Arc};

use rstest::rstest;
use tracing::subscriber::with_default;

use crate::sections::content_filter_records::{
    ContentFilterAction, ContentFilterCategoryConfig, ContentFilterSeverity,
};

use super::loading::load_categories;

#[test]
fn load_categories_skips_failures_and_keeps_valid_records() {
    super::tests::load_categories_resolves_custom_category_files();
    super::tests::partial_updates_survive_later_load_failure();
}

#[rstest]
#[case::empty("", true)]
#[case::traversal("../sample", true)]
#[case::punctuation("sample!", true)]
#[case::unicode("sample\u{e9}", true)]
#[case::disabled("sample", false)]
fn load_categories_rejects_invalid_or_disabled(#[case] name: &str, #[case] enabled: bool) {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("sample.yaml");
    fs::write(&path, "keywords: [{keyword: danger, severity: high}]\n").unwrap();
    let config = ContentFilterCategoryConfig {
        category: name.to_owned(),
        enabled,
        action: ContentFilterAction::Block,
        severity_threshold: ContentFilterSeverity::Low,
        category_file: Some(path.display().to_string()),
    };
    let loaded = load_categories(
        &[config],
        &[temp.path().to_path_buf()],
        temp.path(),
        &super::Settings::default(),
    );
    assert!(loaded.loaded_categories.is_empty());
    assert!(loaded.category_keywords.is_empty());
}

#[rstest]
#[case::low(ContentFilterSeverity::Low, true)]
#[case::medium(ContentFilterSeverity::Medium, false)]
#[case::high(ContentFilterSeverity::High, false)]
fn load_categories_filters_both_maps(
    #[case] threshold: ContentFilterSeverity,
    #[case] includes_low: bool,
) {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("sample.yaml");
    fs::write(&path, "keywords: [{keyword: Lower, severity: low}, {keyword: Upper, severity: high}]\nalways_block_keywords: [{keyword: Always}, {keyword: Mild, severity: low}]\n").unwrap();
    let config = ContentFilterCategoryConfig {
        category: "Mixed_Name-1".to_owned(),
        enabled: true,
        action: ContentFilterAction::Mask,
        severity_threshold: threshold,
        category_file: Some(path.display().to_string()),
    };
    let loaded = load_categories(
        &[config],
        &[temp.path().to_path_buf()],
        temp.path(),
        &super::Settings::default(),
    );
    let expected = (
        "Mixed_Name-1".to_owned(),
        "high".to_owned(),
        ContentFilterAction::Mask,
    );
    assert_eq!(loaded.category_keywords.get("upper"), Some(&expected));
    assert_eq!(
        loaded.always_block_category_keywords.get("always"),
        Some(&expected)
    );
    assert_eq!(loaded.category_keywords.contains_key("lower"), includes_low);
    assert_eq!(
        loaded.always_block_category_keywords.contains_key("mild"),
        includes_low
    );
    assert!(!loaded.category_keywords.contains_key("always"));
    assert!(!loaded.always_block_category_keywords.contains_key("upper"));
    assert!(loaded.loaded_categories.contains_key("Mixed_Name-1"));
    assert!(loaded.conditional_categories.is_empty());
}

#[test]
fn load_categories_warns_when_the_file_is_missing() {
    let temp = tempfile::tempdir().unwrap();
    let log = tempfile::NamedTempFile::new().unwrap();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_writer(Arc::new(log.reopen().unwrap()))
        .finish();
    let config = ContentFilterCategoryConfig {
        category: "missing".to_owned(),
        enabled: true,
        action: ContentFilterAction::Block,
        severity_threshold: ContentFilterSeverity::Medium,
        category_file: Some(temp.path().join("missing.yaml").display().to_string()),
    };
    let root = temp.path().to_path_buf();
    let loaded = with_default(subscriber, || {
        load_categories(
            &[config],
            slice::from_ref(&root),
            &root,
            &super::Settings::default(),
        )
    });
    assert!(loaded.loaded_categories.is_empty());
    let output = fs::read_to_string(log.path()).unwrap();
    assert!(output.contains("WARN"));
    assert!(output.contains("missing.yaml"));
    assert!(!output.contains("ERROR"));
}
