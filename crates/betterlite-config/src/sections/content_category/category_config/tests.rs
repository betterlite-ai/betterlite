#![cfg(test)]

use crate::sections::content_filter_records::category_types::CategoryFileData;
use rstest::rstest;
use std::{fs, sync::Arc};
use tracing::subscriber::with_default;

#[test]
fn category_config_constructor_covers_defaults_and_normalization() {
    super::tests::category_config_default_action_is_block();
    super::tests::category_config_default_action_mask_is_kept();
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

#[rstest]
#[case::global("(?a)^spam$", "SPAM", "\u{17f}pam")]
#[case::scoped("^(?a:spam)$", "sPaM", "\u{17f}pam")]
#[case::global_non_ascii("(?a)^\u{e9}$", "\u{e9}", "\u{c9}")]
#[case::scoped_non_ascii("^(?a:\u{e9})$", "\u{e9}", "\u{c9}")]
#[case::nested_unicode(r"(?a:(?u:^\w+$))", "\u{e9}", "!")]
#[case::restored_unicode("^(?a:spam)spam$", "SPAM\u{17f}pam", "\u{17f}pamspam")]
#[case::combined_ascii_case("(?ai)^spam$", "SPAM", "\u{17f}pam")]
#[case::dot(r"(?a)danger.*zone", "DANGER middle ZONE", "danger\nzone")]
#[case::negated_space(r"(?a)\S+", "abc", " \t ")]
#[case::negated_class(r"(?a)[^x]", "a", "x")]
#[case::ascii_dot_is_one_character(r"(?a)^.$", "\u{e9}", "ab")]
#[case::scoped_ascii_dot(r"^(?a:.)$", "\u{e9}", "ab")]
#[case::ascii_dot_counted(r"(?a)a.{2}b", "a\u{e9}\u{e9}b", "a\u{e9}b")]
#[case::ascii_non_space(r"(?a)^\S$", "\u{e9}", " ")]
#[case::ascii_non_word_ignores_case(r"(?a)^\W$", "\u{17f}", "s")]
#[case::ascii_non_digit(r"(?a)^\D$", "\u{e9}", "1")]
#[case::unicode_non_word_after_flag_group(r"(?m)^\W$", "!", "\u{e9}")]
#[case::unicode_non_digit(r"^\D$", "a", "\u{663}")]
#[case::unicode_non_space(r"^\S$", "a", "\u{a0}")]
#[case::multiline_lines(r"(?m)^foo$", "foo\nbar", "foox\nbar")]
#[case::multiline_before_newline(r"(?m)foo$", "foo\n", "foox\n")]
#[case::ascii_multiline(r"(?am)^foo$", "foo\nbar", "foox\nbar")]
#[case::multiline_scope_ends(r"(?m:a$)\nb$", "a\nb\n", "a\nb\nc")]
#[case::multiline_turned_off(r"(?m)(?-m:a$)", "a\n", "a\nb")]
#[case::end_of_text(r"foo$", "foo", "foo\nbar")]
#[case::before_final_newline(r"foo$", "foo\n", "foo\nbar")]
#[case::absolute_end(r"foo\Z", "foo", "foo\n")]
#[case::dollar_in_class(r"[$]", "$", "x")]
#[case::class_then_end(r"[x]foo$", "xfoo\n", "xfoo\nbar")]
#[case::escaped_dollar(r"\$", "$", "x")]
fn python_syntax_matches_re_semantics(
    #[case] source: &str,
    #[case] matching: &str,
    #[case] excluded: &str,
) {
    let mut data: CategoryFileData = serde_saphyr::from_str("keywords: []").unwrap();
    data.phrase_patterns.push(source.to_owned());
    let category = super::model::CategoryConfig::from_data(data).unwrap();
    assert_eq!(category.phrase_patterns.len(), 1, "must retain {source}");
    let pattern = &category.phrase_patterns[0];
    assert!(pattern.regex.is_match(matching).unwrap(), "{source}");
    assert!(!pattern.regex.is_match(excluded).unwrap(), "{source}");
}

#[rstest]
#[case::turn_off_ascii("(?-a)word")]
#[case::turn_off_unicode("(?-u)word")]
#[case::incompatible_ascii_unicode("(?au)word")]
fn python_flag_removals_python_rejects_are_skipped(#[case] source: &str) {
    let mut data: CategoryFileData = serde_saphyr::from_str("keywords: []").unwrap();
    data.phrase_patterns.push(source.to_owned());
    let category = super::model::CategoryConfig::from_data(data).unwrap();
    assert!(category.phrase_patterns.is_empty(), "{source}");
}

#[rstest]
#[case::lookahead(r"(?a)secret\s*(?=:)")]
#[case::scoped_lookahead(r"(?a:secret)(?=:)")]
#[case::backreference(r"(?a)(\w)\1")]
fn python_ascii_patterns_with_backtracking_only_syntax_are_skipped(#[case] source: &str) {
    let mut data: CategoryFileData = serde_saphyr::from_str("keywords: []").unwrap();
    data.phrase_patterns.push(source.to_owned());
    let category = super::model::CategoryConfig::from_data(data).unwrap();
    assert!(category.phrase_patterns.is_empty(), "{source}");
}

#[rstest]
#[case::negated_class(r"(?a)^[^x]$", "a")]
#[case::non_word_in_class(r"(?a)^[\W]$", "!")]
fn python_ascii_classes_match_single_bytes_of_non_ascii_text(
    #[case] source: &str,
    #[case] ascii: &str,
) {
    let mut data: CategoryFileData = serde_saphyr::from_str("keywords: []").unwrap();
    data.phrase_patterns.push(source.to_owned());
    let category = super::model::CategoryConfig::from_data(data).unwrap();
    let pattern = &category.phrase_patterns[0];
    assert!(pattern.regex.is_match(ascii).unwrap(), "{source}");
    assert!(!pattern.regex.is_match("\u{e9}").unwrap(), "{source}");
}

#[test]
fn python_invalid_branch_reset_warns_and_skips_only_invalid_pattern() {
    let data = serde_saphyr::from_str(
        "category_name: syntax\nphrase_patterns: ['(?|danger)', '(?a)warning']",
    )
    .unwrap();
    let log = tempfile::NamedTempFile::new().unwrap();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_writer(Arc::new(log.reopen().unwrap()))
        .finish();
    let category = with_default(subscriber, || {
        super::model::CategoryConfig::from_data(data).unwrap()
    });
    assert_eq!(category.phrase_patterns.len(), 1);
    assert_eq!(category.phrase_patterns[0].source, "(?a)warning");
    assert!(
        category.phrase_patterns[0]
            .regex
            .is_match("WARNING")
            .unwrap()
    );
    let output = fs::read_to_string(log.path()).unwrap();
    assert!(output.contains("WARN"));
    assert!(output.contains("syntax"));
    assert!(output.contains("(?|danger)"));
}

#[test]
fn oniguruma_named_group_syntax_is_retained() {
    let mut data: CategoryFileData = serde_saphyr::from_str("keywords: []").unwrap();
    data.phrase_patterns.push("(?<name>danger)".to_owned());
    let category = super::model::CategoryConfig::from_data(data).unwrap();
    assert_eq!(category.phrase_patterns.len(), 1);
    assert!(
        category.phrase_patterns[0]
            .regex
            .is_match("danger")
            .unwrap()
    );
}

#[test]
fn literal_ascii_flag_text_inside_a_class_is_rewritten() {
    let mut data: CategoryFileData = serde_saphyr::from_str("keywords: []").unwrap();
    data.phrase_patterns.push("(?a)[x(?a)]".to_owned());
    let category = super::model::CategoryConfig::from_data(data).unwrap();
    assert_eq!(category.phrase_patterns.len(), 1);
    let pattern = &category.phrase_patterns[0];
    assert!(pattern.regex.is_match("x").unwrap());
    assert!(pattern.regex.is_match("b").unwrap());
}
