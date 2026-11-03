#![cfg(test)]

#[test]
fn partial_updates_preserve_prior_successful_loads() {
    super::tests::partial_updates_survive_later_load_failure();
}

use crate::sections::content_filter_records::ContentFilterCategoryConfig;
use rstest::rstest;
use std::{fs, sync::Arc};
use tracing::subscriber::with_default;

#[rstest]
#[case::regular_severity("keywords", "{keyword: broken}")]
#[case::regular_keyword("keywords", "{severity: high}")]
#[case::always_keyword("always_block_keywords", "{severity: high}")]
fn malformed_entry_preserves_only_prior_updates(#[case] field: &str, #[case] broken: &str) {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("partial.yaml");
    fs::write(&path, format!("{field}: [{{keyword: before, severity: high}}, {broken}, {{keyword: after, severity: high}}]\n")).unwrap();
    if field == "always_block_keywords" {
        use std::io::Write as _;
        fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(b"keywords: [{keyword: regular, severity: high}]\n")
            .unwrap();
    }
    let config: ContentFilterCategoryConfig = serde_saphyr::from_str(&format!(
        "category: partial\naction: BLOCK\ncategory_file: {}\n",
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
    assert!(loaded.loaded_categories.contains_key("partial"));
    if field == "always_block_keywords" {
        assert!(loaded.category_keywords.is_empty());
    }
    let words = if field == "keywords" {
        loaded.category_keywords
    } else {
        loaded.always_block_category_keywords
    };
    assert_eq!(
        words.keys().map(String::as_str).collect::<Vec<_>>(),
        ["before"]
    );
    let output = fs::read_to_string(log.path()).unwrap();
    assert!(output.contains("ERROR"));
    assert!(output.contains("partial"));
}
