#![cfg(test)]

use crate::sections::content_filter_records::category_types::CategoryFileData;
use rstest::rstest;

#[test]
fn category_config_constructor_covers_defaults_and_normalization() {
    super::tests::category_config_default_action_is_block();
    super::tests::category_config_rejects_invalid_action();
    super::tests::category_config_lowercases_words_and_skips_invalid_regex();
}

#[rstest]
#[case::global(r"(?a)\bdanger\b", "\u{e9}DANGER\u{e9}")]
#[case::scoped(r"(?a:\bdanger\b)", "\u{e9}DANGER\u{e9}")]
#[case::unicode_after_scope(r"(?a:\bdanger\b) \w+$", "DANGER \u{e9}")]
fn python_ascii_patterns_are_retained(#[case] source: &str, #[case] text: &str) {
    let mut data: CategoryFileData = serde_saphyr::from_str("keywords: []").unwrap();
    data.phrase_patterns.push(source.to_owned());
    let category = super::model::CategoryConfig::from_data(data).unwrap();
    assert_eq!(category.phrase_patterns.len(), 1);
    let pattern = &category.phrase_patterns[0];
    assert_eq!(pattern.source, source);
    assert!(pattern.regex.is_match(text).unwrap());
    assert!(!pattern.regex.is_match("endangered").unwrap());
}
